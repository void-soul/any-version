use futures_util::{Stream, StreamExt};
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub(super) async fn copy_limited<S, B, E, W>(
    mut stream: S, output: &mut W, limit: u64, mut check: impl FnMut() -> Result<(), String>,
) -> Result<u64, String>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>, E: std::fmt::Display,
    W: AsyncWrite + Unpin,
{
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        check()?;
        let chunk = chunk.map_err(|error| format!("读取下载失败: {error}"))?;
        let bytes = chunk.as_ref();
        total = total.checked_add(bytes.len() as u64).ok_or("下载大小溢出")?;
        if total > limit { return Err(format!("下载文件超过大小限制（{limit} 字节）")); }
        output.write_all(bytes).await.map_err(|error| format!("写入下载失败: {error}"))?;
    }
    check()?;
    output.flush().await.map_err(|error| format!("写入下载失败: {error}"))?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn streaming_limit_does_not_write_overflow_chunk() {
        for (limit, succeeds, expected) in [(6, true, "abcdef"), (5, false, "abc")] {
            let (mut writer, mut reader) = tokio::io::duplex(64);
            let stream = futures_util::stream::iter([Ok::<_, String>(b"abc"), Ok(b"def")]);
            let result = copy_limited(stream, &mut writer, limit, || Ok(())).await;
            assert_eq!(result.is_ok(), succeeds);
            drop(writer);
            let mut bytes = String::new();
            reader.read_to_string(&mut bytes).await.unwrap();
            assert_eq!(bytes, expected);
        }
    }

    #[tokio::test]
    async fn streaming_propagates_failure_and_cancellation() {
        let mut writer = tokio::io::sink();
        let stream = futures_util::stream::iter([Err::<Vec<u8>, _>("broken body")]);
        assert!(copy_limited(stream, &mut writer, 64, || Ok(())).await.unwrap_err().contains("broken body"));
        let stream = futures_util::stream::iter([Ok::<_, String>(b"abc")]);
        assert_eq!(copy_limited(stream, &mut writer, 64, || Err("cancelled".into())).await.unwrap_err(), "cancelled");
    }
}
