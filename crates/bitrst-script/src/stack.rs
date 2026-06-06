//! Script evaluation stack.

use crate::ScriptError;

/// Stack of byte vectors used during script execution.
#[derive(Debug, Clone, Default)]
pub struct Stack {
    items: Vec<Vec<u8>>,
}

impl Stack {
    /// Creates an empty stack.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Pushes bytes onto the stack.
    pub fn push(&mut self, data: Vec<u8>) {
        self.items.push(data);
    }

    /// Pops the top item, or returns an error when empty.
    pub fn pop(&mut self) -> Result<Vec<u8>, ScriptError> {
        self.items.pop().ok_or(ScriptError::StackUnderflow)
    }

    /// Returns a copy of the top item without removing it.
    pub fn top(&self) -> Result<&[u8], ScriptError> {
        self.items
            .last()
            .map(Vec::as_slice)
            .ok_or(ScriptError::StackUnderflow)
    }

    /// Duplicates the top stack item.
    pub fn dup(&mut self) -> Result<(), ScriptError> {
        let top = self.top()?.to_vec();
        self.push(top);
        Ok(())
    }

    /// Returns true when the top item represents a truthy script boolean.
    pub fn top_is_true(&self) -> Result<bool, ScriptError> {
        let top = self.top()?;
        Ok(script_bool(top))
    }
}

/// Interprets a stack item as a Bitcoin script boolean.
pub fn script_bool(bytes: &[u8]) -> bool {
    for (i, byte) in bytes.iter().enumerate() {
        if *byte != 0 {
            if i == bytes.len() - 1 && *byte == 0x80 {
                return false;
            }
            return true;
        }
    }
    false
}
