# pi-rs

[English](README.md) ｜ 简体中文

[![CI](https://github.com/123123213weqw/pi-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/123123213weqw/pi-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-4cc38a)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-stable-4cc38a?logo=rust)

> [pi](https://github.com/earendil-works/pi) 编码代理的**行为克隆版**，用 Rust 重写。
> 不是随手重造——**186/186 行为原子对 pi main@35dfa7573（v1.4.0 + 221 提交）逐一验证一致**，场景对场景打平。

| | pi 0.74.2（需 Node ≥ 22.19） | **pi-rs** |
|---|---|---|
| 安装体积 | 197 MB（node_modules） | **2.5 MB 单个静态二进制** |
| 峰值内存（真实任务） | 161–201 MB | **8–63 MB（约 13–21× 更低）** |
| 冷启动 | 910 ms | **2 ms（约 455×）** |
| 运行时依赖 | Node.js | **无** |
| Arena 对局¹ | 102/104（98.1%） | **102/104——同一道题、同一种挂法** |

¹ harness-arena：52 题 × 2 轮，同一模型（deepseek-chat），同一台机器。完整数据见 [docs/bench.md](docs/bench.md)。

Agent 的墙钟时间 99% 花在等模型上——换语言买不到速度，但能买到**密度和分发形态**：
**16 GB 内存塞得下约 455 个并发 agent 会话**（Node 版只有约 88 个），单文件部署，零安装。
这就是 pi-rs 存在的理由。

## 为什么是"克隆"而不是"叉子"

上面每个数字都可复现。pi-rs 对齐真 pi 分三层验证：

1. **行为原子（186/186 一致）**。两个二进制打同一个脚本化 mock LLM，各自发出的**每个请求全部落盘**，
   逐原子 diff——系统提示词（逐字节）、工具 schema、wire 编码、请求参数、stdout、退出码、
   会话 JSONL、文件系统终态。见 [tests/atom](tests/atom)。
2. **Arena 对拍**。作为一等适配器接入 harness-arena，与真 pi 0.74.2 同场竞技。
   102/104 打平；唯一失手的 `hell-01` 两臂**同题同挂**。
3. **占用**。每道 arena 任务都用 `/usr/bin/time -v` 采集峰值 RSS 与墙钟。
   [docs/bench.md](docs/bench.md)。

两处已记录的偏离：
- pi v1.4.0 对 DeepSeek 直通模型已改发 `system`（旧版的 `developer` 角色会被真 API 422 拒绝），pi-rs 行为一致；
- **同一次 `edit` 调用里出现两个相同 `oldText` 会让上游 pi 崩溃**（未处理异常 → 自动重试 → 失败尝试被 `ContextEditEntry` 从上下文切除）。pi-rs 返回优雅的 `edits[0] and edits[1] overlap …` 错误——严格更有用，原子套件钉死了我们的行为。

## 快速上手

```bash
cargo build --release          # → target/release/pi，2.5 MB 单二进制
./target/release/pi "refactor src/foo.rs"               # 交互式 TUI（带初始消息）
./target/release/pi -p "refactor src/foo.rs"            # 一次性 print 模式
./target/release/pi --mode json -p "list todos"         # JSON 事件流
./target/release/pi -t read,grep,find,ls -p "review"    # 只读工具集
DEEPSEEK_API_KEY=sk-... ./target/release/pi -p "hi"
```

供应商：`deepseek`（默认）、`openai`，或任意 OpenAI 兼容端点——`--api-key` +
目录 id（`deepseek-chat`、`deepseek-reasoner` 等）或 `provider/model` 直通。

## 里面有什么

```
┌────────────────────────────────────────────────────────┐
│ pi (CLI)                                               │
│  ├─ args.rs          CLI 参数（自 args.ts 逐条移植）      │
│  ├─ main.rs          print / json / 一次性 三种模式       │
│  ├─ agent.rs         agent 循环（toolUse ⇄ toolResult）  │
│  ├─ provider.rs      OpenAI 兼容 SSE 流式                │
│  ├─ session.rs       会话 JSONL，header v3，条目树        │
│  ├─ system_prompt.rs 逐字节一致的提示词构建器              │
│  ├─ context.rs       AGENTS.md / CLAUDE.md 发现           │
│  └─ tools/           7 个内置工具                        │
└────────────────────────────────────────────────────────┘
```

**7 个内置工具**，自 pi 0.74.2 逐行移植——同名、同 JSON schema、同截断
（2000 行 / 50 KB）、同错误文案：

`read`（offset/limit 续读提示）· `bash`（尾部截断、完整输出落盘逃生口）·
`edit`（多块精确替换 + NFKC 模糊兜底，BOM/CRLF 感知）· `write` ·
`grep`（ripgrep 后端，`path:line: text`）· `find`（fd 后端，v8 兼容）·
`ls`（大小写不敏感排序，`/` 后缀）。默认启用 `read bash edit write`；
只读三件套经 `--tools read,grep,find,ls` 激活。

会话文件与 pi **直接互换**：同 header（`type:"session", version:3`）、
同条目树（8 位 id、parentId 链）、同 usage 记账（`input = prompt − cached`），
arena 式记账读两个 harness 无需改一行。

## 实现笔记

- 无异步运行时：单线程事件循环 + 阻塞式 SSE——agent 循环本来就是串行的。
- 转录以 JSONL 躺在磁盘上，内存只持有下一次请求需要的那部分。
- 依赖刻意收敛到五个 crate：`serde`、`serde_json`（preserve_order——模型发来的
  键序原样回传）、`ureq`（rustls）、`uuid`、`unicode-normalization`。
- edit 工具的 diff/模糊匹配引擎是 pi `edit-diff.ts` 的逐行移植（含 NFKC
  智能引号/破折号归一化）；`truncate.rs` 逐位复刻 pi 的 head/tail 截断，
  含 UTF-8 边界处理。

## 复现结论

```bash
# 行为原子（需同一台机器装好钉版 UPSTREAM 指向的真 pi + pi-rs，无需 API key；
# 可用 PI_BIN / PIRS_BIN 覆盖二进制路径）
bash tests/atom/run_atom.sh                     # → 186/186 原子一致

# 真模型上的占用 + 任务对拍
bash tests/parity_v100.sh <tasks-dir> pirs <task-ids>
bash tests/parity_v100.sh <tasks-dir> pi  <task-ids>

# 无 key 的快速端到端（脚本化 mock provider）
bash tests/e2e_mock.sh
```

数字、方法论与分题明细：[docs/bench.md](docs/bench.md)。

## 对齐范围

**已对齐**：headless 核心——agent 循环、7 工具、会话格式、CLI 面
（`-p`、`--mode json`、`--provider/--model/--api-key/--tools/--thinking/--session-dir/…`）、
系统提示词、上下文文件发现、usage/成本记账——**以及交互式 TUI**：全屏转录视图
（流式助手文本、内联工具行、工具输出折叠）、底部输入行带历史、Esc/Ctrl+C 中断、
`/exit`——并支持**流式 CoT**（thinking 增量实时以暗灰斜体渲染）与 `Ctrl+O` 展开工具
输出。已用 tmux 程序化发键完成端到端验证。v1.4.0 的 wire 层也已移植：分节系统
提示词（`<tools>`/`<rules>`/`<docs>`/`<cwd>`）、strict 模式工具 schema、`max_tokens`
参数、会话 `role:"system"` 分节条目。与 pi TUI 的差距：自动补全、模型切换器、主题、
鼠标支持、图片渲染。

**路线图**：extensions & skills 加载 · MCP 客户端 · compaction ·
`--export` HTML · RPC 模式 · Anthropic 原生 API。（pi-durable 的事务化运行时与
chord 插件宿主暂不在范围内。）

## 致意与许可

pi-rs 是 [pi](https://github.com/earendil-works/pi)（Mario Zechner / badlogic /
earendil-works）的衍生作品——全部设计功力属于原作。这个仓库回答一个问题而非替代
原版：*当模型是瓶颈时，harness 的运行时让你付出多少？* 想要完整的 pi 体验
——TUI、extensions、skills——请用 [pi 本尊](https://github.com/earendil-works/pi)。

MIT。见 [LICENSE](LICENSE)。
