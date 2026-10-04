//! 最小的 SSE 解析（OpenAI 兼容流式接口只用 `data:` 字段）。

/// 按块喂入字节，取出完整事件的 `data` 内容。多行 `data:` 用换行拼接；注释行和其他字段忽略。
#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        // JSON 载荷里的换行都是转义过的，直接丢掉 \r 就能统一 \r\n 与 \n 两种分隔
        self.buf
            .extend(chunk.iter().copied().filter(|&b| b != b'\r'));
        let mut out = Vec::new();
        while let Some(end) = find(&self.buf, b"\n\n") {
            let event: Vec<u8> = self.buf.drain(..end + 2).collect();
            let text = String::from_utf8_lossy(&event[..end]);
            let data: Vec<&str> = text
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            if !data.is_empty() {
                out.push(data.join("\n"));
            }
        }
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_events_across_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(b"data: {\"a\":").is_empty());
        assert_eq!(p.push(b"1}\n\ndata: [DONE]\n"), vec!["{\"a\":1}"]);
        assert_eq!(p.push(b"\n"), vec!["[DONE]"]);
    }

    #[test]
    fn handles_crlf_comments_and_multiline_data() {
        let mut p = SseParser::default();
        let got = p.push(b": keep-alive\r\n\r\nevent: x\r\ndata: a\r\ndata: b\r\n\r\ndata:c\n\n");
        assert_eq!(got, vec!["a\nb", "c"]);
    }

    #[test]
    fn utf8_split_inside_a_character_is_kept() {
        let mut p = SseParser::default();
        let bytes = "data: 晴\n\n".as_bytes();
        assert!(p.push(&bytes[..7]).is_empty());
        assert_eq!(p.push(&bytes[7..]), vec!["晴"]);
    }
}
