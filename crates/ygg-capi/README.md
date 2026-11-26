# ygg-capi — `ygg/1` 协议的 C ABI

一条连接 = 一个世界实例。`ygg_session_new` 装载一份**包镜像**，之后所有交互都是
「一段 JSON 进、一个 JSON 帧数组出」。没有结构体过境，没有回调，没有 out 参数。
头文件：[`include/ygg.h`](include/ygg.h)（与导出面逐字一致）。

产物：`target/<profile>/libygg_capi.{dylib,so,a}`（`cdylib` + `staticlib`）。
Windows 下静态库为 `ygg_capi.lib`；C 侧链接时把 `include/` 加进包含路径即可。

## 15 行示例

```c
#include <stdio.h>
#include <string.h>
#include "ygg.h"

int main(void) {
    const char *image = "{\"modSource\":\"world.seal()\",\"manifest\":\"\"}";
    ygg_session_t *s = ygg_session_new((const uint8_t *)image, strlen(image));
    if (!s) { fprintf(stderr, "load: %s\n", ygg_last_error()); return 1; }
    char *reply = ygg_session_handle(s, "{\"type\":\"hello\",\"protocol\":\"ygg/1\","
                                         "\"codec\":\"json\",\"pack_id\":\"ygg:demo\","
                                         "\"pack_hash\":\"<manifest hash>\"}");
    puts(reply);                       /* [{"type":"ready",...}] */
    ygg_string_free(reply);
    ygg_session_free(s);
    return 0;
}
```

`pack_hash` 是 manifest 文本的 SHA-256 十六进制串（`ygg_host::Loaded::manifest_hash`）：
宿主先从包清单算出它，握手才对得上。第一次握手前只接受 `hello`——这正是 `error`
里 `E_PROTOCOL`（重连）/ `E_PACK`（换包）两类分开的原因。

## 三条边界规则

1. **进来的字节是 UTF-8，出去的是调用方所有的字符串**：`ygg_session_handle` /
   `ygg_last_error` 的返回值一律 `ygg_string_free` 释放（不要 `free(3)`——
   分配器必须成对）。`ygg_protocol_version()` 返回静态区，**不要**释放。
2. **协议错误不用 NULL 表达**：回执里就是一帧
   `{"type":"error","code":"E_STATE","msg":"…"}`。NULL 只表示「分配不出来」、
   「`msg_json` 为空 / 不是 UTF-8」或「会话指针为空」——后三种仍会给出可解析的
   `error` 帧（不是 NULL），NULL 之后用 `ygg_last_error()` 取原因。
3. **一个句柄只在一个线程上用**：句柄内部可变（`&mut Session`），并发进入同一
   句柄是未定义行为。要并发，一连接一会话。

## 这个 crate 里没有协议逻辑

全部判断只有三类：指针空不空、字节是不是 UTF-8、分配成不成功。消息语义（状态门、
`pack_hash` 对账、`seal` 检查、帧切分、错误分类）都在 `crates/ygg-host`。绑定层每加
一个 `if`，就多一份与 Rust 宿主对不上的协议实现——`tests/parity.rs` 与
`crates/ygg-wasm/tests/parity.rs` 跑同一份
[`packages/schema/protocol/parity-script.json`](../../packages/schema/protocol/parity-script.json)，
逐帧钉住三条载体（Rust `Session` / WASM `YggSession` / C ABI）行为等价。

## 验证

```bash
cargo test -p ygg-capi            # C 面：装载、握手、坏输入、NULL 会话、释放
cargo test -p ygg-capi --test parity   # 三载体逐帧等价
```
