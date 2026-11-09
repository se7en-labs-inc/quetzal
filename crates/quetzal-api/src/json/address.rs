use quetzal_primitives::{AddressEntry, AddressState};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AddressResponse {
    pub id: String,
    pub state: String,
    pub derivation_path: Vec<String>,
}

impl AddressResponse {
    pub fn from_entry(entry: &AddressEntry) -> Self {
        let state = match entry.state {
            AddressState::Used => "used",
            AddressState::Unused => "unused",
        };
        let role_index = entry.path.role.index();
        let derivation_path = vec![
            "1852H".to_string(),
            "1815H".to_string(),
            format!("{}H", entry.path.account_ix),
            format!("{}", role_index),
            format!("{}", entry.path.address_ix),
        ];
        AddressResponse {
            id: entry.address.0.clone(),
            state: state.to_string(),
            derivation_path,
        }
    }
}
