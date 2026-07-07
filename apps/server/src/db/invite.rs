//! Invites: single-use, expiring tokens that bind a public key to a user.
//! Only the SHA-256 of a token is stored; the token itself lives in the blob.

use libsql::params;
use sha2::{Digest, Sha256};
use std::io;

use super::{Db, db_error, text_column};

impl Db {
    pub(crate) async fn create_invite(
        &self,
        user_id: &str,
        token_hash: &str,
        now: u64,
        expires_at: u64,
    ) -> io::Result<()> {
        self.conn
            .execute(
                "INSERT INTO invites (token_hash, user_id, expires_at, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                (token_hash, user_id, expires_at as i64, now as i64),
            )
            .await
            .map_err(db_error)?;

        Ok(())
    }

    /// Consumes an unredeemed, unexpired invite and binds the public key to
    /// its user. Returns the user id (the key id) on success.
    pub(crate) async fn redeem_invite(
        &self,
        token_hash: &str,
        public_key: &str,
        now: u64,
    ) -> io::Result<Option<String>> {
        // The single UPDATE is the atomic redeem gate: only one request can
        // flip redeemed_at from NULL.
        let consumed = self
            .conn
            .execute(
                "UPDATE invites SET redeemed_at = ?1
                 WHERE token_hash = ?2 AND redeemed_at IS NULL AND expires_at > ?1",
                (now as i64, token_hash),
            )
            .await
            .map_err(db_error)?;

        if consumed == 0 {
            return Ok(None);
        }

        let mut rows = self
            .conn
            .query(
                "SELECT user_id FROM invites WHERE token_hash = ?1",
                params![token_hash],
            )
            .await
            .map_err(db_error)?;
        let row = rows
            .next()
            .await
            .map_err(db_error)?
            .ok_or_else(|| io::Error::other("redeemed invite row disappeared"))?;
        let user_id = text_column(&row, 0)?;

        self.conn
            .execute(
                "UPDATE users SET public_key = ?1 WHERE id = ?2",
                (public_key, user_id.as_str()),
            )
            .await
            .map_err(db_error)?;

        Ok(Some(user_id))
    }
}

pub(crate) fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
