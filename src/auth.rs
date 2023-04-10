pub mod user;

use crate::auth::user::User;
use anyhow::{anyhow, Result};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use bincode::{Decode, Encode};
use dashmap::DashMap;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

pub type SessionToken = [u8; 16];

/// Represents the internal authentication state for a session.
#[derive(Clone, Debug, PartialEq)]
pub enum AuthState {
    Unauthenticated,
    Authenticated(String),
}

/// Represents an authentication message sent by the client.
#[derive(Encode, Decode, Debug)]
pub enum AuthClientMessage {
    Authentication(String, String),
    SessionToken(SessionToken),
}

/// Represents an authentication message sent by the server.
#[derive(Encode, Decode)]
pub enum AuthServerMessage {
    Authenticated(Vec<u8>, Vec<u8>, SessionToken),
    Ok,
    Failed,
}

/// Represents a module providing basic authentication functionality.
pub struct Auth {
    users: DashMap<String, User>,
    hasher: Argon2<'static>,
}

impl Auth {
    /// Creates a new instance of the authentication module.
    ///
    /// ### Arguments
    /// - `users` - a map of users (username -> `User`)
    pub fn new(users: DashMap<String, User>) -> Self {
        Self {
            users,
            hasher: Argon2::default(),
        }
    }

    /// Authenticates the given users and returns a session token if successful.
    ///
    /// ### Arguments
    /// - `username` - the username
    /// - `password` - the password
    ///
    /// ### Returns
    /// - `Bytes` containing the session token
    pub async fn authenticate(&self, username: &str, password: String) -> Result<SessionToken> {
        let user = self
            .users
            .get(username)
            .ok_or_else(|| anyhow!("Unknown user: {username}"))?;
        let password_hash = PasswordHash::new(user.password_hash()).map_err(|err| {
            anyhow!("Could not parse user password hash for user '{username}': {err}")
        })?;

        self.hasher
            .verify_password(password.as_bytes(), &password_hash)
            .map_err(|err| anyhow!("Could not verify credentials for user '{username}': {err}"))?;

        Ok(user.new_session().await)
    }

    /// Verifies the given session token for the specified user.
    ///
    /// ### Arguments
    /// - `username` - the username
    /// - `session_token` - the session token
    ///
    /// ### Returns
    /// - `true` if the session token is valid, `false` otherwise
    pub fn verify_session_token(
        &self,
        username: &str,
        session_token: SessionToken,
    ) -> Result<bool> {
        let user = self
            .users
            .get(username)
            .ok_or_else(|| anyhow!("Unknown user: {username}"))?;

        Ok(user.check_session_validity(session_token))
    }

    /// Resets all user sessions.
    pub fn reset(&self) {
        for entry in self.users.iter() {
            entry.value().reset();
        }
    }

    /// Loads the contents of a file with users and their passwords hashes into a map.
    ///
    /// ### Arguments
    /// - `users_file` - path to the users file
    ///
    /// ### Returns
    /// - `DashMap` containing all loaded users
    pub fn load_users_file(users_file: &Path) -> Result<DashMap<String, User>> {
        let file = File::open(users_file)?;
        let lines = BufReader::new(file).lines();

        let result: DashMap<String, User> = DashMap::new();

        for line in lines {
            let user: User = line?.try_into()?;
            result.insert(user.username().clone(), user);
        }

        Ok(result)
    }

    /// Writes the users and their password hashes into the specified file
    ///
    /// ### Arguments
    /// - `users_file` - path to the users file
    /// - `users` - a map of users (username -> `User`)
    pub fn save_users_file(users_file: &Path, users: DashMap<String, User>) -> Result<()> {
        if users_file.exists() {
            fs::remove_file(users_file)?;
        }

        let file = File::create(users_file)?;
        let mut writer = BufWriter::new(file);

        for (username, user) in users {
            writer.write_all(format!("{username}:{}\n", user.password_hash()).as_bytes())?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::auth::user::User;
    use crate::auth::Auth;
    use argon2::password_hash::rand_core::OsRng;
    use argon2::password_hash::SaltString;
    use argon2::{Argon2, PasswordHasher};
    use dashmap::DashMap;

    #[test]
    fn test_authentication() {
        let users: DashMap<String, User> = DashMap::new();

        let argon = Argon2::default();
        let username = "test".to_owned();
        let password = "password".to_owned();
        let salt = SaltString::generate(&mut OsRng);

        let password_hash = argon.hash_password(password.as_bytes(), &salt).unwrap();

        let test_user = User::new(username.clone(), password_hash.to_string());
        users.insert(username.clone(), test_user);

        let auth = Auth::new(users);
        let session_token = tokio_test::block_on(auth.authenticate(&username, password))
            .expect("Credentials are valid");
        assert!(auth
            .verify_session_token(&username, session_token)
            .expect("User exists"))
    }
}
