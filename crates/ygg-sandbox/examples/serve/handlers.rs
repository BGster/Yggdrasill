use super::*;

// ============================================================================
// §数据端点（第一级 / 第二级 / 存档数据 API）
// ============================================================================

/// `GET /api/runs`——第一级：runs 枚举（注册表扫描，行带根标识）。
pub(crate) fn handle_runs(shared: &ServeShared) -> Response {
    // 外层循环根、内层循环局；个别根不可读 → errors 记录、其余根照常出行
    // （观察面「局部真」优于「全黑」，整体仍 200）。
    let roots = shared.roots_snapshot();
    let mut rows: Vec<(u64, Json)> = Vec::new();
    let mut errors: Vec<Json> = Vec::new();
    for root in &roots {
        // 单局根只产出自身一行（run.json 在 = 局在）；父目录根枚举其子目录。
        let candidates: Vec<PathBuf> = if root.single_run {
            if root.path.join(crate::META_FILE).is_file() {
                vec![root.path.clone()]
            } else {
                Vec::new()
            }
        } else {
            match std::fs::read_dir(&root.path) {
                Ok(entries) => entries
                    .flatten()
                    .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
                    .map(|entry| entry.path())
                    .collect(),
                Err(err) => {
                    errors.push(json!({
                        "root": root.path.display().to_string(),
                        "message": format!("读取失败：{err}"),
                    }));
                    continue;
                }
            }
        };
        for run_dir in candidates {
            let meta_path = run_dir.join(crate::META_FILE);
            // run.json 缺失 → 跳过（中止创建的半壳目录不是局）。
            if !meta_path.is_file() {
                continue;
            }
            let run_id = run_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (meta, meta_error) = match std::fs::read_to_string(&meta_path)
                .map_err(|err| err.to_string())
                .and_then(|text| serde_json::from_str::<Json>(&text).map_err(|err| err.to_string()))
            {
                Ok(meta) => (meta, None),
                Err(err) => (Json::Null, Some(err)),
            };
            let updated_at = meta["updated_at"].as_u64().unwrap_or(0);
            let mut row = json!({
                "runId": run_id,
                "root": root.path.display().to_string(),
                "meta": meta,
                "coord": crate::read_json(&run_dir.join(crate::COORDINATION_FILE)),
            });
            if let Some(meta_error) = meta_error {
                row["metaError"] = json!(meta_error);
            }
            match resolve_backend(&run_dir) {
                Ok(backend) => {
                    row["backend"] = json!(backend.as_str());
                    row["probe"] = observe::save_probe(&run_dir, backend)
                        .map(|probe| {
                            json!({
                                "generation": probe.generation,
                                "latestId": probe.latest_id,
                                "latestCreatedAt": probe.latest_created_at,
                                "count": probe.count,
                            })
                        })
                        .unwrap_or_else(|err| json!({"count": 0, "probeError": err.to_string()}));
                }
                Err(err) => {
                    row["backendError"] = json!(err);
                    row["probe"] = json!({"generation": null, "latestId": null, "latestCreatedAt": 0, "count": 0});
                }
            }
            row["panel"] = panel_json(
                &run_id,
                run_mod_dir(&meta, Some(root.path.as_path())).as_deref(),
            );
            rows.push((updated_at, row));
        }
    }
    // updated_at 降序（活跃局自然在前）；meta 解析失败的行（键 0）沉底。
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    let truncated = rows.len() > RUNS_CAP;
    rows.truncate(RUNS_CAP);
    let mut body = json!({
        "version": shared.broadcaster.version(),
        "generatedAt": now_secs(),
        "roots": roots
            .iter()
            .map(|root| json!(root.path.display().to_string()))
            .collect::<Vec<_>>(),
        "runs": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
    });
    if truncated {
        body["truncated"] = json!(true);
    }
    if !errors.is_empty() {
        body["errors"] = json!(errors);
    }
    Response::json(200, &body)
}

/// `GET /api/runs/<id>/saves?limit=`——第二级：存档列表（§3.3）。
pub(crate) fn handle_saves(shared: &ServeShared, run_id: &str, query: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    let limit = match query_usize(query, "limit", 0, observe::SAVE_LIST_MAX_LIMIT) {
        Ok(limit) => limit,
        Err(response) => return response,
    };
    let saves = match observe::save_list(&run_dir, backend, limit) {
        Ok(saves) => saves,
        Err(err) => return error_response(500, "observe_failed", &err.to_string()),
    };
    let probe = observe::save_probe(&run_dir, backend).unwrap_or_default();
    let meta = crate::read_meta(&run_dir.join(crate::META_FILE));
    let panel = panel_json(
        run_id,
        meta.as_ref()
            .and_then(|meta| run_mod_dir(meta, run_dir.parent()))
            .as_deref(),
    );
    Response::json(
        200,
        &json!({
            "runId": run_id,
            "backend": backend.as_str(),
            "generation": probe.generation,
            "saves": saves.iter().map(save_row).collect::<Vec<_>>(),
            "panel": panel,
        }),
    )
}

pub(crate) fn save_row(meta: &observe::SaveMeta) -> Json {
    json!({
        "id": meta.id,
        "generation": meta.generation,
        "seq": meta.seq,
        "createdAt": meta.created_at,
        "bytes": meta.bytes,
        "latest": meta.latest,
    })
}

/// `GET /api/runs/<id>/saves/<sid>`——存档数据 API：原始字节 + `X-Ygg-*` 元数据头。
/// body = 存档原字节（**零包装零重序**）。
pub(crate) fn handle_save_blob(shared: &ServeShared, run_id: &str, sid: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    if !valid_save_id(sid) {
        return error_response(
            400,
            "bad_save_id",
            &format!("存档 id `{sid}` 非法（只接受 `latest` 或十进制 id）"),
        );
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    match observe::save_get(&run_dir, backend, sid) {
        Ok(Some((meta, bytes))) => Response {
            status: 200,
            reason: "OK",
            content_type: "application/json",
            headers: vec![
                ("X-Ygg-Run", run_id.to_owned()),
                ("X-Ygg-Save-Id", meta.id),
                (
                    "X-Ygg-Generation",
                    meta.generation
                        .map_or(String::new(), |value| value.to_string()),
                ),
                ("X-Ygg-Seq", meta.seq.to_string()),
                ("X-Ygg-Created-At", meta.created_at.to_string()),
                ("X-Ygg-Bytes", meta.bytes.to_string()),
            ],
            body: bytes,
        },
        Ok(None) => error_response(404, "save_not_found", &format!("存档 `{sid}` 不存在")),
        Err(err) => error_response(500, "observe_failed", &err.to_string()),
    }
}

/// `GET /api/runs/<id>/events?since=&limit=`——事件日志分页（seq 升序；payload 原文）。
pub(crate) fn handle_events(shared: &ServeShared, run_id: &str, query: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    let since = match query_usize(query, "since", 0, usize::MAX) {
        Ok(since) => since as u64,
        Err(response) => return response,
    };
    let limit = match query_usize(
        query,
        "limit",
        EVENT_LIMIT_DEFAULT as usize,
        EVENT_LIMIT_MAX as usize,
    ) {
        Ok(limit) => limit as u32,
        Err(response) => return response,
    };
    let page = match observe::event_page(&run_dir, backend, since, limit) {
        Ok(page) => page,
        Err(err) => return error_response(500, "observe_failed", &err.to_string()),
    };
    let probe = observe::save_probe(&run_dir, backend).unwrap_or_default();
    let records = page
        .records
        .iter()
        .map(|record| {
            json!({
                "seq": record.seq,
                "event": record.event,
                "payload": record.payload, // 原文透传（actor/actor_session 段随 r3 自动出现）
                "ok": record.ok,
                "timestamp": record.timestamp,
                "ending": record.ending,
            })
        })
        .collect::<Vec<_>>();
    Response::json(
        200,
        &json!({
            "runId": run_id,
            "generation": probe.generation,
            "latest": page.latest,
            "since": since,
            "records": records,
        }),
    )
}

/// `GET /api/runs/<id>/artifacts`——当前代制品元信息（path 升序）。
pub(crate) fn handle_artifacts(shared: &ServeShared, run_id: &str) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    match observe::artifact_list_view(&run_dir, backend) {
        Ok(artifacts) => Response::json(
            200,
            &json!({
                "artifacts": artifacts.iter().map(|meta| json!({
                    "path": meta.path,
                    "bytes": meta.bytes,
                    "sha256": meta.sha256,
                    "updatedAt": meta.updated_at,
                })).collect::<Vec<_>>(),
            }),
        ),
        Err(err) => error_response(500, "observe_failed", &err.to_string()),
    }
}

/// `GET /api/runs/<id>/artifacts/<path..>`——制品原文（`validate_artifact_path` 门复用；
/// 读侧封顶 1 MiB → 413）。
pub(crate) fn handle_artifact_content(
    shared: &ServeShared,
    run_id: &str,
    artifact_path: &str,
) -> Response {
    if let Err(response) = check_run_id(run_id) {
        return response;
    }
    if let Err(err) = ygg_store::validate_artifact_path(artifact_path) {
        return error_response(400, "bad_artifact_path", &err.to_string());
    }
    let run_dir = match locate_run_dir(shared, run_id) {
        Ok(run_dir) => run_dir,
        Err(response) => return response,
    };
    let backend = match resolve_backend(&run_dir) {
        Ok(backend) => backend,
        Err(err) => return error_response(500, "backend_unresolved", &err),
    };
    match observe::artifact_get_view(&run_dir, backend, artifact_path) {
        Ok(Some(record)) => {
            if record.content.len() > ARTIFACT_HTTP_CAP {
                return error_response(413, "artifact_too_large", "制品超过读侧封顶 1 MiB");
            }
            Response {
                status: 200,
                reason: "OK",
                content_type: content_type_for(artifact_path),
                headers: vec![
                    ("X-Ygg-Run", run_id.to_owned()),
                    ("X-Ygg-Bytes", record.content.len().to_string()),
                    ("X-Ygg-Sha256", record.meta.sha256),
                ],
                body: record.content,
            }
        }
        Ok(None) => error_response(
            404,
            "artifact_not_found",
            &format!("制品 `{artifact_path}` 不存在"),
        ),
        Err(err) => error_response(500, "observe_failed", &err.to_string()),
    }
}
