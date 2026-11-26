/*
 * ygg.h — Yggdrasill `ygg/1` 协议的 C ABI（ygg-capi）。
 *
 * 一条连接 = 一个世界实例：`ygg_session_new` 装载包镜像，之后所有交互都是
 * 「一段 JSON 进、一段 JSON 数组出」。没有结构体过境，没有回调，没有 out 参数。
 *
 * 边界规则
 *   1. 进的是 UTF-8 字节、NUL 结尾字符串；出的是**调用方所有**的字符串，
 *      一律用 `ygg_string_free` 释放（不要 `free(3)`）。
 *   2. 协议错误不用 NULL 表达：回执里就是一帧 `{"type":"error","code":…,"msg":…}`。
 *      NULL 只意味着「分配不出来」或「会话/入参指针不合法」，原因见 `ygg_last_error`。
 *   3. 一个句柄只能在一个线程上使用（内部可变，并发进入同一句柄是未定义行为）。
 */
#ifndef YGG_H
#define YGG_H

#include <stddef.h>
#include <stdint.h>

// pi-lens-ignore: no-reserved-identifiers
#ifdef __cplusplus
extern "C" {
#endif

/* 不透明会话句柄。 */
typedef struct ygg_session ygg_session_t;

/* 协议版本常量：始终是 "ygg/1"（静态区，不要释放）。 */
const char *ygg_protocol_version(void);

/* 包镜像（JSON 文本，pack_len 字节，无需 NUL 结尾）→ 未握手会话。
 * 失败返回 NULL，原因见 ygg_last_error()。 */
ygg_session_t *ygg_session_new(const uint8_t *pack, size_t pack_len);

/* 一条消息 JSON → 帧流 JSON（一个数组：step 两帧，其余一帧）。
 * 返回值由调用方用 ygg_string_free 释放；NULL 仅表示分配失败。 */
char *ygg_session_handle(ygg_session_t *s, const char *msg_json);

/* 释放本库返回的字符串（NULL 允许）。 */
void ygg_string_free(char *s);

/* 释放会话（NULL 允许）。 */
void ygg_session_free(ygg_session_t *s);

/* 最近一次「只能返回 NULL」的失败原因：调用方所有（ygg_string_free 释放）；
 * 无记录时返回 NULL。 */
char *ygg_last_error(void);

// pi-lens-ignore: no-reserved-identifiers
#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* YGG_H */
