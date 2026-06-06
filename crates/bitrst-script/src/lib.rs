//! Bitcoin Script interpreter (P2PKH subset for M5).

mod interpreter;
mod opcodes;
mod p2pkh;
mod stack;

pub use interpreter::{verify_script, ScriptError};
pub use p2pkh::{p2pkh_script_pubkey, p2pkh_script_sig, push_data};
