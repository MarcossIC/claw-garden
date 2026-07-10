use super::PermissionMode;
use super::errors::ToolErrorMessage;

pub struct PermissionEnforcer;

impl PermissionEnforcer {
    /// Returns Ok if the agent's mode satisfies the tool's required permission.
    pub fn check(
        tool_name: &str,
        required: PermissionMode,
        agent_mode: PermissionMode,
    ) -> Result<(), ToolErrorMessage> {
        if required > agent_mode {
            Err(ToolErrorMessage::permission_denied(
                tool_name,
                &format!("{:?}", required),
                &format!("{:?}", agent_mode),
            ))
        } else {
            Ok(())
        }
    }
}
