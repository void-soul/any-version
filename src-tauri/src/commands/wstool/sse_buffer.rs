//! Linear incremental SSE framing with a bounded unfinished event (including its partial line).
pub(super) struct SseBuffer { event: String, line: Vec<u8>, limit: usize, skip_lf: bool }
impl SseBuffer {
    pub(super) fn new(limit: usize) -> Self { Self { event: String::new(), line: Vec::new(), limit, skip_lf: false } }
    pub(super) fn feed(&mut self, chunk: &[u8], mut emit: impl FnMut(String)) -> Result<(), String> {
        for &byte in chunk {
            if self.skip_lf { self.skip_lf = false; if byte == b'\n' { continue; } }
            if self.event.len() + self.line.len() + 1 > self.limit { return Err("SSE事件超过1 MiB，已断开连接".into()); }
            if byte != b'\n' && byte != b'\r' { self.line.push(byte); continue; }
            self.skip_lf = byte == b'\r';
            if self.line.is_empty() {
                if !self.event.trim().is_empty() { self.event.push('\n'); emit(std::mem::take(&mut self.event)); }
                else { self.event.clear(); }
            } else {
                let line = String::from_utf8_lossy(&self.line);
                if self.event.len() + line.len() + 1 > self.limit { return Err("SSE事件超过1 MiB，已断开连接".into()); }
                self.event.push_str(&line); self.event.push('\n');
            }
            self.line.clear();
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_crlf_and_multibyte_utf8_across_arbitrary_chunks() {
        let mut parser = SseBuffer::new(1024); let mut events = Vec::new();
        for chunk in "data: 中文\r\ndata: value\r\n\r\n".as_bytes().chunks(1) { parser.feed(chunk, |event| events.push(event)).unwrap(); }
        assert_eq!(events, vec!["data: 中文\ndata: value\n\n"]);
    }
    #[test]
    fn unfinished_lines_and_events_share_the_same_budget() {
        let mut parser = SseBuffer::new(12);
        parser.feed(b"data: 123\n", |_| {}).unwrap();
        assert!(parser.feed(b"1234", |_| {}).is_err());
        let mut parser = SseBuffer::new(3);
        assert!(parser.feed(b"long", |_| {}).is_err());
    }
    #[test]
    fn event_budget_resets_after_dispatch_and_blank_lines_are_not_events() {
        let mut parser = SseBuffer::new(16); let mut count = 0;
        for _ in 0..100 { parser.feed(b"\n\ndata: x\n\n", |_| count += 1).unwrap(); }
        assert_eq!(count, 100);
    }
    #[test]
    fn accepts_bare_cr_line_endings_as_well_as_crlf_and_lf() {
        let mut parser = SseBuffer::new(1024); let mut events = Vec::new();
        for chunk in [b"data: a\r".as_slice(), b"\rdata: b\n\n"] { parser.feed(chunk, |event| events.push(event)).unwrap(); }
        assert_eq!(events, vec!["data: a\n\n", "data: b\n\n"]);
    }

}
