//! Generate an Argon2id PHC hash for `APEX_ADMIN_PASSWORD_HASH` or
//! `WEB_USERS_JSON` entries.
//!
//! Usage:
//!   cargo run -p apex-api --example hash_password -- 'your-password'

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(password) = std::env::args().nth(1) else {
        eprintln!("usage: hash_password <password>");
        return ExitCode::from(2);
    };

    match apex_api::web::auth::hash_password(&password) {
        Ok(hash) => {
            println!("{hash}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("failed to hash password: {err}");
            ExitCode::FAILURE
        }
    }
}
