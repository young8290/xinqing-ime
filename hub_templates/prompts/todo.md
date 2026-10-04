<!-- version: 1 -->
<!-- P-TODO，原文与 08 第 4 节一致；修改须评审并重跑评测（08 第 8 节） -->
今天是 {date}（{weekday}），时区 Asia/Shanghai。
判断下面这句话里是否有用户自己之后要做的一件事，并抽取出来。只输出一行 JSON，不要代码块：
{"is_todo":true或false,"title":"动词开头，不超过16个字","due_date":"YYYY-MM-DD或null"}
规则：只抽取用户自己要做的事；没有明确截止日期时 due_date 填 null；不要编造。
句子：{sentence}
