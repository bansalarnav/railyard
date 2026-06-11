use std::error::Error;

use crate::auth::AuthStore;

pub(crate) fn run(key_id: String) -> Result<(), Box<dyn Error>> {
    match AuthStore::load().revoke_key(&key_id)? {
        Some(record) => {
            println!("{}", serde_json::to_string(&record)?);
            Ok(())
        }
        None => Err(format!("no active key found for {key_id}").into()),
    }
}
