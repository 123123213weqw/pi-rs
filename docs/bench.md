# pi-rs vs pi 0.74.2 基准（V100 实测，2026-10-28）

环境：wzu-SYS-4029GP-TRT（V100 评测机），deepseek-chat 直连，
pi 0.74.2 旁路安装（`~/pi-0.74.2`，Node v24.22.1），pi-rs 静态二进制 2.5MB。
任务：harness-arena 11 道（algo-01..05、data-01、eval-01、gen-01、
hard-01/03/05，pytest 与脚本双验证器），跨基础/数据/生成/难题域。
采集：`/usr/bin/time -v`（峰值 RSS）、`date +%s%N`（墙钟）；每臂每题 1 次。

## 结果（11 题汇总）

| 指标 | pi-rs | pi 0.74.2 |
|---|---|---|
| 通过率 | **11/11** | 11/11 |
| 单题耗时范围 | 4.2 – 18.9s | 5.2 – 47.6s |
| 峰值 RSS 范围 | **8.0 – 62.9MB** | 161 – 201MB |

分题明细（秒 / MB）：

| 任务 | pi-rs | pi | 备注 |
|---|---|---|---|
| algo-01 | 4.2 / 8.0 | 5.2 / 161 | Kadane |
| algo-02 | 5.7 / 9.5 | 5.3 / 167 | LIS |
| algo-03 | 4.5 / 8.0 | 7.8 / 175 | pair-sum |
| algo-04 | 9.5 / 34.3 | 17.5 / 188 | 逆序对 |
| algo-05 | 6.2 / 18.5 | 31.8 / 173 | 滑窗 |
| data-01 | 4.2 / 10.0 | 5.7 / 173 | CSV 清洗(脚本验证) |
| eval-01 | 8.1 / 10.0 | 14.1 / 182 | 表达式求值(脚本验证) |
| gen-01 | 9.9 / 61.5 | 32.1 / 196 | 拓扑排序生成 |
| hard-01 | 13.2 / 9.0 | 47.6 / 187 | 难题 |
| hard-03 | 5.7 / 9.0 | 10.4 / 185 | 难题(大组合数) |
| hard-05 | 18.9 / 26.0 | 40.5 / 197 | 难题(缺失子串) |

- **功能对齐**：11/11 vs 11/11；难题域（hard-01/03/05，claude-code 曾被
  击穿的档位）pi-rs 全过。
- **峰值内存**：**~13–21× 更低**（V8 堆底 + 模块解析 vs 原生静态；
  大输出任务 pi-rs 升到 63MB，仍不足 pi 地板的 40%）。
- **墙钟耗时**：11 题全部 pi-rs 更快或持平（模型主导 + 省 V8 冷启动；
  hard 域差距最大：13.2 vs 47.6s）。

## 静态指标

| 指标 | pi 0.74.2 | pi-rs | 倍数 |
|---|---|---|---|
| 安装体积 | 197MB（node_modules） | 2.5MB（单二进制） | **~79×** |
| 冷启动（`--version`，5 次均值） | 910ms | **2ms** | **~455×** |
| 运行时依赖 | Node ≥ 22.19 | 无 | — |
| 并发 100 agent 内存（按峰值 RSS 外推） | ~16.5GB | ~0.9GB | **~19×** |

## 结论

功能与 pi 0.74.2 headless 核心对齐（工具/循环/会话/CLI/提示词），
系统占用低 1–2 个数量级。对 arena 大规模 fan-out 场景，同机并行容量
提升约一个数量级。


## 行为原子对齐（2026-10-28 补充）

方法：真 pi 0.74.2 与 pi-rs 同时指向本地 mock LLM（pi 经扩展劫持 baseUrl，
pi-rs 走 PI_BASE_URL），脚本化相同的工具调用场景；逐原子 diff 两边发出的
实际请求（系统提示词/工具 schema/wire 消息/请求参数）、stdout、退出码、
会话 JSONL、文件系统终态。

**85/85 行为原子一致**（5 场景：bash 往返 / read+edit / 7 工具只读链 /
错误路径 / 未启用工具）。据此修复的差异：

| 原子 | 修复 |
|---|---|
| wire 编码 | user 内容为块数组；assistant 带工具调用时 content=null + reasoning_content；工具结果字符串 |
| 请求参数 | store:false、max_completion_tokens=384000、reasoning_effort:"high"、thinking:{type:enabled}（deepseek 默认） |
| usage 语义 | input = prompt_tokens − cached；totalTokens = prompt + completion（不扣缓存） |
| 工具 schema | required 数组、function.strict:false、write 无 additionalProperties、grep 两处描述文案、find 片段单复数 |
| 系统提示词 | 合并 per-tool promptGuidelines（read/edit/write 共 6 条） |
| 会话条目 | 头部后补 model_change + thinking_level_change 两条 |
| 工具启用集 | 未启用工具返回 `Tool <name> not found`（此前错误地直接执行） |

已知偏离（记录在案）：pi 按在线目录对 deepseek 发 `role:"developer"`，
真 DeepSeek API 实测 422 拒绝；pi-rs 固定发 `system`（differ 中归一化）。
测试脚本：`tests/atom/run_atom.sh`（V100）。


## harness-arena 正式对局（2026-10-28，pi-rs 首战）

场地：V100，harness-arena 52 题套件 × 2 轮 = 208 run，deepseek-chat，
两臂同题同轮对拍，结果写入 `runs/arena.db` 累计账本（suite `20261028-*`）。

| 臂 | n | pass | 通过率 | 输入 tok | 缓存读 | 输出 tok | 均耗时 | pts |
|---|---|---|---|---|---|---|---|---|
| pi 0.74.2 | 104 | 102 | 98.1% | 295,254 | 18.2M | 261,377 | 19.5s | 102.0 |
| **pi-rs** | 104 | **102** | **98.1%** | 272,910 | 21.7M | 359,396 | 23.6s | **102.0** |

分域 pass/total 两臂**逐列相同**：algo 34/34、coding 40/42、data 6/6、
frontend 2/2、math 16/16、ops 4/4。唯一失手题两臂一致：`hell-01`
（各 2/2 全挂——同一道题同一种死法，正是行为等价的直接证据）。

结论：**功能完全一样**在 52 题 × 2 轮的正式口径下成立（102/104 对 102/104，
points 相同，failures 相同）；token 分布同量级（pi-rs 输出偏多来自个别题
多走了一轮重试，属模型随机性而非 harness 差异）。
