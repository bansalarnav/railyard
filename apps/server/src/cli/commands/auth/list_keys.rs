use std::error::Error;

use crate::auth::AuthStore;

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let keys = AuthStore::load().list_keys()?;
    println!("{}", serde_json::to_string_pretty(&keys)?);
    Ok(())
}
