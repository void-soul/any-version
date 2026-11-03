use super::*;

fn decode(chunks: &[&[u8]]) -> Vec<String> {
    let mut parser = SseBuffer::default(); let mut result = Vec::new();
    for chunk in chunks {
        parser.push(chunk);
        while let Some(block) = parser.next_block().unwrap() {
            if let Some(data) = extract_sse_data(&block) { result.push(data); }
        }
    }
    assert!(parser.finish().unwrap().is_none());
    result
}

#[test]
fn unicode_and_mixed_line_endings_survive_every_split_point() {
    let bytes = "\u{feff}: heartbeat\r\ndata: {\"text\":\r\ndata: \"中文😀\"}\r\n\r\ndata: second\n\ndata: third\r\r".as_bytes();
    let expected = vec!["{\"text\":\n\"中文😀\"}".to_string(), "second".into(), "third".into()];
    for cut in 0..=bytes.len() { assert_eq!(decode(&[&bytes[..cut], &bytes[cut..]]), expected, "cut={cut}"); }
    for size in 1..=bytes.len() { assert_eq!(decode(&bytes.chunks(size).collect::<Vec<_>>()), expected, "size={size}"); }
}

#[test]
fn data_fields_keep_empty_lines_and_only_remove_one_optional_space() {
    assert_eq!(extract_sse_data("data: first\ndata:\ndata:  third"), Some("first\n\n third".into()));
    assert_eq!(extract_sse_data(": comment\nevent: message\ndata"), Some(String::new()));
    assert_eq!(extract_sse_data(": data: not a field\nevent: message"), None);
    assert_eq!(extract_sse_data("data: [DONE]"), Some("[DONE]".into()));
}

#[test]
fn invalid_utf8_is_an_error_instead_of_silently_discarded_bytes() {
    let mut parser = SseBuffer::default(); parser.push(b"data: \xff\n\n");
    assert!(parser.next_block().is_err());
    let mut parser = SseBuffer::default(); parser.push(b"data: \xe4\xb8");
    assert!(parser.next_block().unwrap().is_none());
    assert!(parser.finish().is_err());
}

#[test]
fn undelimited_tail_is_exposed_only_for_explicit_caller_validation() {
    let mut parser = SseBuffer::default(); parser.push(b"data: {\"x\":1}");
    assert!(parser.next_block().unwrap().is_none());
    assert_eq!(extract_sse_data(&parser.finish().unwrap().unwrap()), Some("{\"x\":1}".into()));
    assert!(parser.finish().unwrap().is_none());
}
