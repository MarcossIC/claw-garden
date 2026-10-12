//! Sentence frames for synthetic rows. Placeholders: `{a}` and `{b}` take an
//! identifier, `{w1}`..`{w3}` take a topic word. A frame is chosen by
//! `(memory_type, language)` so a distractor reads like the row type it
//! imitates instead of forming a cluster of its own.

/// Frames for a distractor of `memory_type`. Any language other than Spanish
/// falls back to English, and an unlisted type to `semantic_fact`.
pub fn distractor_frames(memory_type: &str, spanish: bool) -> &'static [&'static str] {
    match (memory_type, spanish) {
        ("chat_message", false) => &[
            "Quick note: `{a}` came up again together with {w1} and {w2}; we left `{b}` as is and will revisit {w3}.",
            "Asked about {w1} in `{a}`; the answer was that {w2} depends on `{b}` and on {w3}.",
        ],
        ("chat_message", true) => &[
            "Nota rápida: `{a}` volvió a aparecer junto con {w1} y {w2}; dejamos `{b}` como estaba y revisamos {w3} después.",
            "Se consultó por {w1} en `{a}`; la respuesta fue que {w2} depende de `{b}` y de {w3}.",
        ],
        ("tool_error", false) => &[
            "Error while handling `{a}`: {w1} failed after {w2}; retrying `{b}` did not fix {w3}.",
            "The run stopped at `{a}` with a {w1} problem. It looked related to {w2} in `{b}`, not to {w3}.",
        ],
        ("tool_error", true) => &[
            "Error al procesar `{a}`: {w1} falló después de {w2}; reintentar `{b}` no resolvió {w3}.",
            "La ejecución se detuvo en `{a}` con un problema de {w1}. Parecía ligado a {w2} en `{b}`, no a {w3}.",
        ],
        ("code_block", false) => &[
            "// `{a}`: {w1} then {w2}\nlet value = `{b}`; // keeps {w3}",
            "// handles {w1} for `{a}`\nreturn `{b}`.apply({w2}); // {w3}",
        ],
        ("code_block", true) => &[
            "// `{a}`: {w1} y luego {w2}\nlet valor = `{b}`; // conserva {w3}",
            "// maneja {w1} para `{a}`\nreturn `{b}`.aplicar({w2}); // {w3}",
        ],
        ("session_summary", false) => &[
            "Session summary: reviewed `{a}` and {w1}, adjusted {w2} around `{b}`, and left {w3} for later.",
            "We worked on {w1} in `{a}`, compared it with `{b}`, and agreed to keep {w2} and {w3} unchanged.",
        ],
        ("session_summary", true) => &[
            "Resumen de sesión: se revisó `{a}` y {w1}, se ajustó {w2} alrededor de `{b}` y se dejó {w3} para después.",
            "Trabajamos en {w1} dentro de `{a}`, lo comparamos con `{b}` y acordamos mantener {w2} y {w3} sin cambios.",
        ],
        ("user_preference", false) => &[
            "The user prefers {w1} over {w2} when working on `{a}`, and wants `{b}` to keep {w3}.",
            "Preference noted: for `{a}` use {w1}; for `{b}` avoid {w2} and {w3}.",
        ],
        ("user_preference", true) => &[
            "El usuario prefiere {w1} antes que {w2} al trabajar en `{a}`, y quiere que `{b}` conserve {w3}.",
            "Preferencia anotada: para `{a}` usar {w1}; para `{b}` evitar {w2} y {w3}.",
        ],
        ("architectural_decision", false) => &[
            "Decision: `{a}` will own {w1}; `{b}` keeps {w2}. The alternative of changing {w3} was discarded.",
            "We chose to route {w1} through `{a}` instead of `{b}`, accepting a cost in {w2} to protect {w3}.",
        ],
        ("architectural_decision", true) => &[
            "Decisión: `{a}` se encarga de {w1}; `{b}` conserva {w2}. Se descartó la alternativa de cambiar {w3}.",
            "Elegimos canalizar {w1} por `{a}` en lugar de `{b}`, aceptando un costo en {w2} para proteger {w3}.",
        ],
        (_, false) => &[
            "`{a}` relates to {w1} and {w2}; `{b}` is the place where {w3} is configured.",
            "In `{a}`, {w1} is derived from {w2}; `{b}` exposes {w3} to the rest of the system.",
        ],
        (_, true) => &[
            "`{a}` se relaciona con {w1} y {w2}; `{b}` es donde se configura {w3}.",
            "En `{a}`, {w1} se deriva de {w2}; `{b}` expone {w3} al resto del sistema.",
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentKind {
    Function,
    ErrorCode,
}

/// A row a lexical family repeats with only the identifier changing, and the
/// question that asks for that identifier.
pub struct LexicalFrame {
    pub memory_type: &'static str,
    pub content: &'static str,
    pub query: &'static str,
}

pub fn lexical_frames(kind: IdentKind, spanish: bool) -> &'static [LexicalFrame] {
    match (kind, spanish) {
        (IdentKind::Function, false) => &[
            LexicalFrame {
                memory_type: "code_block",
                content: "fn `{id}` normalizes the incoming payload and hands it to the caller without touching the cache.",
                query: "what does {id} do",
            },
            LexicalFrame {
                memory_type: "code_block",
                content: "Helper `{id}` retries the request twice before it reports the failure upstream.",
                query: "where is {id} used",
            },
        ],
        (IdentKind::Function, true) => &[
            LexicalFrame {
                memory_type: "code_block",
                content: "La función `{id}` normaliza la carga recibida y se la devuelve al llamador sin tocar la caché.",
                query: "qué hace {id}",
            },
            LexicalFrame {
                memory_type: "code_block",
                content: "El helper `{id}` reintenta la petición dos veces antes de informar el fallo hacia arriba.",
                query: "dónde se usa {id}",
            },
        ],
        (IdentKind::ErrorCode, false) => &[
            LexicalFrame {
                memory_type: "tool_error",
                content: "The command failed with `{id}` because the upstream service did not answer in time.",
                query: "what does the error {id} mean",
            },
            LexicalFrame {
                memory_type: "tool_error",
                content: "Build aborted: `{id}` was raised while the lock was held.",
                query: "how to fix {id}",
            },
        ],
        (IdentKind::ErrorCode, true) => &[
            LexicalFrame {
                memory_type: "tool_error",
                content: "El comando falló con `{id}` porque el servicio externo no respondió a tiempo.",
                query: "qué significa el error {id}",
            },
            LexicalFrame {
                memory_type: "tool_error",
                content: "Compilación abortada: se lanzó `{id}` mientras el bloqueo estaba tomado.",
                query: "cómo resolver {id}",
            },
        ],
    }
}

/// Aspects of one topic, each stored as its own row. A question about the
/// topic is answered by all of them.
pub fn topic_aspects(spanish: bool) -> &'static [&'static str] {
    if spanish {
        &[
            "El módulo `{key}` es dueño de la política de reintentos y del presupuesto de tiempo.",
            "Quien opera `{key}` entra por el panel de administración y su registro de auditoría.",
            "El despliegue de `{key}` se hace los viernes y exige un plan de reversión.",
        ]
    } else {
        &[
            "The `{key}` module owns the retry policy and the timeout budget.",
            "Operators reach `{key}` through the admin panel and its audit log.",
            "Deployment of `{key}` happens on Fridays and needs a rollback plan.",
        ]
    }
}

pub fn topic_query(spanish: bool) -> &'static str {
    if spanish {
        "todo lo que se sabe sobre {key}"
    } else {
        "everything known about {key}"
    }
}

/// Rows and questions for strict user rules, which carry the top importance.
pub fn rule_frames(spanish: bool) -> &'static [(&'static str, &'static str)] {
    if spanish {
        &[
            (
                "El usuario exige las convenciones `{key}`: aplicarlas siempre antes de proponer cambios.",
                "cuál es la regla del usuario sobre {key}",
            ),
            (
                "Regla firme del usuario para `{key}`: nunca saltarla, ni siquiera en cambios menores.",
                "qué exige el usuario respecto de {key}",
            ),
        ]
    } else {
        &[
            (
                "The user insists on `{key}` conventions: always apply them before proposing any change.",
                "what is the user rule about {key}",
            ),
            (
                "Firm user rule for `{key}`: never skip it, not even on minor changes.",
                "what does the user require regarding {key}",
            ),
        ]
    }
}

pub const CODENAME_ADJECTIVES: [&str; 10] =
    ["amber", "cobalt", "crimson", "ivory", "jade", "onyx", "scarlet", "teal", "umber", "violet"];
pub const CODENAME_NOUNS: [&str; 10] =
    ["heron", "falcon", "otter", "lynx", "badger", "marten", "plover", "ibex", "wren", "stoat"];

pub const FUNCTION_VERBS: [&str; 8] =
    ["resolve", "parse", "merge", "flush", "rotate", "hydrate", "normalize", "reconcile"];
pub const FUNCTION_NOUNS: [&str; 8] =
    ["Session", "Token", "Snapshot", "Manifest", "Cursor", "Ledger", "Bucket", "Journal"];
pub const ERROR_AREAS: [&str; 8] = ["CACHE", "QUEUE", "AUTH", "SYNC", "INDEX", "LOCK", "ROUTE", "STORE"];

/// Word used when a base row has no term of its own to give.
pub fn fallback_word(spanish: bool) -> &'static str {
    if spanish {
        "elemento"
    } else {
        "element"
    }
}

/// Fills a frame; every slot that appears in `values` is replaced.
pub fn fill(frame: &str, values: &[(&str, &str)]) -> String {
    let mut out = frame.to_string();
    for (slot, value) in values {
        out = out.replace(slot, value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOTS: [&str; 5] = ["{a}", "{b}", "{w1}", "{w2}", "{w3}"];

    const TYPES: [&str; 7] = [
        "chat_message",
        "tool_error",
        "code_block",
        "session_summary",
        "user_preference",
        "architectural_decision",
        "semantic_fact",
    ];

    #[test]
    fn every_memory_type_has_frames_in_both_languages_using_every_core_slot() {
        for t in TYPES {
            for spanish in [false, true] {
                let frames = distractor_frames(t, spanish);
                assert!(!frames.is_empty(), "{t} {spanish}");
                for frame in frames {
                    for slot in ["{a}", "{b}", "{w1}", "{w2}"] {
                        assert!(frame.contains(slot), "{t} {spanish}: '{frame}' lacks {slot}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_lexical_frame_names_its_identifier_in_row_and_query() {
        for kind in [IdentKind::Function, IdentKind::ErrorCode] {
            for spanish in [false, true] {
                for frame in lexical_frames(kind, spanish) {
                    assert!(frame.content.contains("`{id}`"), "{}", frame.content);
                    assert!(frame.query.contains("{id}"), "{}", frame.query);
                }
            }
        }
    }

    #[test]
    fn filling_a_frame_leaves_no_slot_behind() {
        let values = [("{a}", "A"), ("{b}", "B"), ("{w1}", "x"), ("{w2}", "y"), ("{w3}", "z")];
        for t in TYPES {
            for frame in distractor_frames(t, false) {
                let filled = fill(frame, &values);
                assert!(!SLOTS.iter().any(|s| filled.contains(s)), "{filled}");
            }
        }
    }
}
