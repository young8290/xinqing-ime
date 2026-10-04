# 心晴 XinQing

> 懂你心情的 AI 输入陪伴。一个 Windows 中文输入法，加上一位陪伴角色“晴晴”。
> 本仓是开源输入法 [清风 WindInput](https://github.com/huanfeng/WindInput)（MIT）的私有分支。

本仓同时放着清风原有的输入法（`wind_input/`、`wind_tsf/` 等）和心晴新增的部分（Hub、XQP 协议、开发工具、模板、评测）。
目录、构建、测试与协作约定统一写在根目录 [AGENTS.md](../../AGENTS.md)，心晴部分见其中「心晴部分」一节。

## 产品书

需求、设计、测试与计划以产品书为准，产品书**只放在** [young8290/xinqing](https://github.com/young8290/xinqing/tree/main/docs/product-book)，本仓不复制。
代码注释和文档里的“08 FR-AIG-02”“17 第 2.5 节”这类编号都指产品书的章节。

| 你是 | 先读 |
|---|---|
| 新加入的组员 | [00 总览与阅读指南](https://github.com/young8290/xinqing/blob/main/docs/product-book/00_总览与阅读指南.md) → 01 → 02 → 03 |
| 准备写代码 | [14 开发环境搭建](https://github.com/young8290/xinqing/blob/main/docs/product-book/14_开发环境搭建与开发指南.md) → [16 任务分解](https://github.com/young8290/xinqing/blob/main/docs/product-book/16_任务分解与工作量估算.md) → [17 详细设计](https://github.com/young8290/xinqing/blob/main/docs/product-book/17_详细设计说明.md) |
| 负责测试 | [12 测试与验收](https://github.com/young8290/xinqing/blob/main/docs/product-book/12_测试与验收.md) → [18 测试用例与追溯矩阵](https://github.com/young8290/xinqing/blob/main/docs/product-book/18_测试用例清单与追溯矩阵.md) |
| 改模板或词表 | [15 内容资产与模板说明](https://github.com/young8290/xinqing/blob/main/docs/product-book/15_内容资产与模板说明.md) |
| 试用者 | [19 用户手册](https://github.com/young8290/xinqing/blob/main/docs/product-book/19_用户手册.md)、[21 隐私说明与知情同意书](https://github.com/young8290/xinqing/blob/main/docs/product-book/21_隐私说明与知情同意书.md) |

实现与产品书不一致的地方记在 [docs/adr/](../adr/)（0007 起），每份 ADR 的“影响”列出产品书要改的章节。

## 本目录

- [identity.md](identity.md)：身份改造（与官方清风共存），以及合并上游的步骤。
- [handover/](handover/)：各角色的交接文档（任务状态、代码地图、进行中的工作、已知问题），随各自的 PR 更新。
