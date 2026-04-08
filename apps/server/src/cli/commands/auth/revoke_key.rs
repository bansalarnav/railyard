use crate::auth::AuthStore;

pub(crate) fn run(key_id: String) {
    let revoked = AuthStore::load()
        .revoke_key(&key_id)
        .expect("failed to revoke auth key");

    match revoked {
        Some(record) => {
            println!(
                "{}",
                serde_json::to_string(&record).expect("failed to serialize revoked key")
            );
        }
        None => {
            eprintln!("No active key found for {key_id}");
            std::process::exit(1);
        }
    }
}
