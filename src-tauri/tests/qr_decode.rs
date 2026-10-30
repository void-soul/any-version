/// Exercise the same image -> PreparedImage -> detect -> decode path as OTP import.
#[test]
fn qr_image_decodes_after_dependency_upgrade() {
    let rows: Vec<&str> = include_str!("fixtures/qr-rqrr.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect();
    assert_eq!(rows.len(), 21);
    assert!(rows.iter().all(|row| row.len() == 21));
    let scale = 8;
    let quiet = 4;
    let size = (21 + quiet * 2) * scale;
    let image = image::GrayImage::from_fn(size, size, |x, y| {
        let (x, y) = ((x / scale) as i32 - quiet as i32, (y / scale) as i32 - quiet as i32);
        let black = (0..21).contains(&x)
            && (0..21).contains(&y)
            && rows[y as usize].as_bytes()[x as usize] == b'1';
        image::Luma([if black { 0 } else { 255 }])
    });
    let mut prepared = rqrr::PreparedImage::prepare(image);
    let contents: Vec<String> = prepared.detect_grids().into_iter()
        .filter_map(|grid| grid.decode().ok().map(|(_, content)| content))
        .collect();
    assert!(contents.iter().any(|content| content == "rqrr"), "decoded: {contents:?}");
}
