// 验证 UpdateTaskInput 中 scheduled_date 的三态语义：
//   字段缺失      -> None            （不修改日期）
//   字段为 null   -> Some(None)      （清空日期，移入收集箱）
//   字段有值      -> Some(Some(日期)) （改期）
// 前端 TaskEditModal 会显式传 scheduledDate: null 来把任务移入收集箱，
// 若这里退化成 None，会导致"移入收集箱"静默失效。

use serde::Deserialize;

/// 把显式 null 与字段缺失区分开：null -> Some(None)，缺失 -> None。
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTaskInput {
    #[serde(default, deserialize_with = "double_option")]
    scheduled_date: Option<Option<String>>,
}

#[test]
fn absent_field_means_no_change() {
    let v: UpdateTaskInput = serde_json::from_str("{}").unwrap();
    assert_eq!(v.scheduled_date, None, "字段缺失应表示不修改");
}

#[test]
fn explicit_null_means_clear_to_inbox() {
    let v: UpdateTaskInput = serde_json::from_str(r#"{"scheduledDate": null}"#).unwrap();
    assert_eq!(v.scheduled_date, Some(None), "显式 null 应表示清空日期");
}

#[test]
fn concrete_value_means_reschedule() {
    let v: UpdateTaskInput = serde_json::from_str(r#"{"scheduledDate": "2026-07-31"}"#).unwrap();
    assert_eq!(v.scheduled_date, Some(Some("2026-07-31".to_string())));
}
