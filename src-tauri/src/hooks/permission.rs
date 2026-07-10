use tokio::sync::oneshot;

/// Decisión del usuario ante un diálogo de confirmación.
#[derive(Debug, PartialEq, Eq)]
pub enum UserDecision {
    Allow,
    Deny,
}

/// Solicitud de confirmación que el agente envía a la UI.
///
/// El agente construye este struct, pone el `response_tx` en él y lo envía
/// por el canal `permission_tx: mpsc::Sender<PermissionRequest>`.
/// La UI (o la política headless en tests) responde a través del `response_tx`.
pub struct PermissionRequest {
    pub tool_name: String,
    /// The **effective** (post-mutation) input — what the tool will actually
    /// receive if the user approves. MUST NOT be the original input: if Main
    /// hooks mutated the input before raising Ask, the user must see and approve
    /// the final state, not the raw arguments the model produced.
    ///
    /// Using the original input here would mean the user approves X but the
    /// tool executes Y — a silent bait-and-switch.
    pub tool_input: serde_json::Value,
    /// Razón provista por el hook — se muestra al usuario en el diálogo.
    pub reason: Option<String>,
    /// Canal para que la UI responda la decisión del usuario.
    pub response_tx: oneshot::Sender<UserDecision>,
}
