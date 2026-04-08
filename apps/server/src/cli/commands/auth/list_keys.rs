use crate::auth::AuthStore;

pub(crate) fn run() {
    let keys = AuthStore::load()
        .list_keys()
        .expect("failed to list auth keys");

    println!(
        "{}",
        serde_json::to_string_pretty(&keys).expect("failed to serialize auth key list")
    );
}
