//! Windows adapter boundary. Hooking and injection are deliberately absent in M0.

use zonkey_types::EditPlan;

/// Records that an edit plan reached the unimplemented Windows boundary.
///
/// # Errors
///
/// Returns an error when insertion text contains a NUL character.
pub fn validate_plan(plan: &EditPlan) -> Result<(), &'static str> {
    if plan.insert_text.contains('\0') {
        return Err("insert_text must not contain NUL");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_plan;
    use zonkey_types::EditPlan;

    #[test]
    fn rejects_nul_text() {
        let plan = EditPlan {
            delete_graphemes: 0,
            insert_text: "a\0b".into(),
        };
        assert!(validate_plan(&plan).is_err());
    }
}
