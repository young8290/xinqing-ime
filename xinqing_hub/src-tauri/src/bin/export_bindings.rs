//! 生成前端的类型安全封装 `xinqing_hub/src/api/bindings.ts`（10 第 5.1 节：禁止手写 invoke 字符串）。
//!
//! 改了命令或事件后运行：`cargo run -p xinqing-hub --bin export-bindings`，并提交生成的文件。
//! CI 会重新生成并检查没有差异。

use std::path::PathBuf;

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/api/bindings.ts")
        });
    xinqing_hub_lib::export_bindings(&out).expect("导出 TypeScript 绑定失败");
    println!("{}", out.display());
}
