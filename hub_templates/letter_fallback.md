<!-- version: 1 -->
<!-- 心晴 · 周信兜底模板（FR-REV-02）。未同意 ④、大模型不可用或周信预算用完时使用。 -->
<!-- 只填统计数字，不加 AI 标识。占位符由代码填充；某项数据缺失时整句省略（以 {?key} … {/key} 包裹的句子）。 -->
<!-- 用语约束：不说“睡眠质量”“失眠”，作息只说“停止打字的时间”；不说教；不出现禁用词。 -->

你好呀，

这一周你在电脑前平均每天打字 {typing_avg}，辛苦了。{?rest_rate}休息提醒完成了 {rest_rate}，{rest_comment}{/rest_rate}{?water}一共喝了 {water} 次水。{/water}

{?best_slot}这周你在{best_slot}的状态最轻松，{/best_slot}{?hard_slot}{hard_slot}好像更容易累一些。{/hard_slot}{?stop_avg}平均停止打字的时间是 {stop_avg}（只统计这台电脑上的打字时间）。{/stop_avg}

{?done}这周你完成了 {schedules_done} 个日程、{todos_done} 件待办，一件一件都在往前走。{/done}

下周可以试试：{tip}

晴晴
