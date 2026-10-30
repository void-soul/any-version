use super::*;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EnvBackup {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub user_vars: std::collections::HashMap<String, String>,
    pub sys_vars: std::collections::HashMap<String, String>,
}
