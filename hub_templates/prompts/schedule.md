<!-- version: 1 -->
<!-- P-SCHEDULE，原文与 08 第 4 节一致；修改须评审并重跑评测（08 第 8 节） -->
今天是 {date}（{weekday}），时区 Asia/Shanghai。
从下面这句话中抽取用户要做的未来事项。只输出一行 JSON，不要代码块，不要解释：
{"has_event":true或false,"title":"不超过12个字","date":"YYYY-MM-DD或null",
 "time":"HH:mm或null","end_time":"HH:mm或null","all_day":true或false,
 "location":"地点或null","is_deadline":true或false}
规则：不确定的字段填 null；不要编造地点；“截止、DDL、之前交”属于 is_deadline=true。
句子：{sentence}
