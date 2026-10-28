# pi-rs

pi coding agent 的 Rust 复刻，对齐 `@earendil-works/pi-coding-agent` **v0.74.2** 的
headless 核心：功能一致、系统占用低一个数量级。

```
┌────────────────────────────────────────────────────────┐
│ pi (CLI)                                               │
│  ├─ args.rs          CLI 参数（逐条对齐 args.ts）        │
│  ├─ main.rs          模式分发：print / json / 一次性     │
│  ├─ agent.rs         agent 循环（对齐 agent-loop.ts）    │
│  ├─ provider.rs      DeepSeek/OpenAI 兼容 SSE 流式       │
│  ├─ session.rs       会话 JSONL（header v3 + message）   │
│  ├─ system_prompt.rs 系统提示词（逐字节对齐）            │
│  ├─ context.rs       AGENTS.md / CLAUDE.md 发现          │
│  └─ tools/           7 个内置工具 + 截断 + edit-diff      │
└────────────────────────────────────────────────────────┘
```

## 为什么是 Rust

agent 的耗时瓶颈在模型侧，harness 本地开销 <1%——语言换不来速度。
语言真正决定的是**系统占用**（RSS / 冷启动 / 安装体积）：

| 指标 | pi 0.74.2 (Node 22+) | pi-rs |
|---|---|---|
| 安装体积 | node_modules ~100MB+ | 单二进制 2.5MB |
| 运行时依赖 | Node ≥ 22.19 | 无 |
| 常驻内存 | V8 堆底 ~40MB + 业务 | 预期 10–30MB |
| 冷启动 | 数百 ms（V8 启动+模块解析） | ~5ms |

（实测数据见下方“基准”。）

## 对齐范围（v0.74.2）

### ✅ 已对齐

- **7 个内置工具**，名称/描述/JSON Schema/输出格式逐字对齐：
  `read`（offset/limit 续读提示）、`bash`（尾部截断 2000 行/50KB、temp file、
  超时/非零退出消息）、`edit`（多块精确替换、NFKC 模糊匹配、BOM/CRLF 感知、
  重叠/重复检测、错误文案）、`write`、`grep`（外部 rg，`path:line: text`、
  500 字符行截断、limit 提示）、`find`（外部 fd，`--full-path` + `**/` 补全，
  v8 兼容回退）、`ls`（大小写不敏感排序、`/` 后缀）。
- **agent 循环**：工具调用顺序执行、toolResult 回填、stopReason 语义
  （stop/toolUse/length/error/aborted）。
- **会话 JSONL**：header `{type:"session",version:3,...}` 首行 + message 条目
  （8 位 id、parentId 链），文件名 `{ISO 时间戳}_{id}.jsonl`，与 pi 会话
  同目录布局；usage 字段（input/output/cacheRead/cacheWrite/totalTokens/cost）
  与 arena 记账口径一致。
- **系统提示词**：默认分支逐字节对齐（工具清单、guidelines、日期/cwd 行）。
- **上下文文件**：`~/.pi/agent/AGENTS.md` 全局 + cwd→根逐级发现。
- **CLI 参数面**：--provider/--model/--api-key/--system-prompt/
  --append-system-prompt/--session-dir/--no-session/--continue/--tools/
  --no-tools/--thinking/--mode json/-p 等，未知 flag 进 diagnostics。
- **print 模式**：text 输出最终 assistant 文本；`--mode json` 输出事件流；
  非 TTY 一次性运行（arena 接法）。

### 🚧 路线图（未对齐）

- 交互式 TUI（pi-tui 差分渲染）
- extensions / skills / prompt templates / themes 加载
- MCP 客户端、OAuth
- compaction（上下文压缩）
- Anthropic 原生 API（当前 DeepSeek/OpenAI 兼容端点优先，与 arena 同口径）
- `--export` HTML、RPC 模式
- 图片附件自动缩放（read 工具图片走 base64 原样附带）

## 开发

```bash
cargo build --release          # 单 crate，无异步运行时
bash tests/e2e_mock.sh         # mock provider 端到端（无需 API key）
bash tests/parity_v100.sh <tasks-dir> <pi|pirs> <task-ids...>   # 真模型对拍
```

依赖刻意最小：`serde` / `serde_json` / `ureq`(rustls) / `uuid` /
`unicode-normalization`。grep/find 复用系统 `rg`/`fd`（同 pi 的 ensureTool
路径），缺失时 find 回退内置 walker。

## 基准

见 `docs/bench.md`（V100 实测：同任务双臂对拍，/usr/bin/time -v 采集 RSS）。
