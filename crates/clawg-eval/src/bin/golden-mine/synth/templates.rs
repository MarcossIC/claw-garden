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
            "Side thread: someone asked whether `{a}` still depends on {w1}; the reply pointed at `{b}` and said {w2} and {w3} can wait.",
            "Chat recap: we looked at {w1} for `{a}`, then switched to {w2}; `{b}` stays untouched until {w3} is clear.",
        ],
        ("chat_message", true) => &[
            "Nota rápida: `{a}` volvió a aparecer junto con {w1} y {w2}; dejamos `{b}` como estaba y revisamos {w3} después.",
            "Se consultó por {w1} en `{a}`; la respuesta fue que {w2} depende de `{b}` y de {w3}.",
            "Hilo lateral: alguien preguntó si `{a}` todavía depende de {w1}; la respuesta señaló `{b}` y dijo que {w2} y {w3} pueden esperar.",
            "Resumen del chat: miramos {w1} para `{a}`, luego pasamos a {w2}; `{b}` no se toca hasta aclarar {w3}.",
        ],
        ("tool_error", false) => &[
            "Error while handling `{a}`: {w1} failed after {w2}; retrying `{b}` did not fix {w3}.",
            "The run stopped at `{a}` with a {w1} problem. It looked related to {w2} in `{b}`, not to {w3}.",
            "Stack trace ended in `{a}` after {w1} timed out; the retry through `{b}` hit the same {w2} and left {w3} half written.",
            "Failure report: `{a}` returned a {w1} error during {w2}, and `{b}` showed the same symptom once {w3} was reloaded.",
        ],
        ("tool_error", true) => &[
            "Error al procesar `{a}`: {w1} falló después de {w2}; reintentar `{b}` no resolvió {w3}.",
            "La ejecución se detuvo en `{a}` con un problema de {w1}. Parecía ligado a {w2} en `{b}`, no a {w3}.",
            "El stack trace terminó en `{a}` después de que {w1} venciera; el reintento por `{b}` chocó con el mismo {w2} y dejó {w3} a medio escribir.",
            "Informe de fallo: `{a}` devolvió un error de {w1} durante {w2}, y `{b}` mostró el mismo síntoma al recargar {w3}.",
        ],
        ("code_block", false) => &[
            "// `{a}`: {w1} then {w2}\nlet value = `{b}`; // keeps {w3}",
            "// handles {w1} for `{a}`\nreturn `{b}`.apply({w2}); // {w3}",
            "// `{a}` wraps {w1}
const result = `{b}`({w2}); // fallback: {w3}",
            "// TODO: move {w1} out of `{a}`
if (`{b}`) { {w2}(); } // {w3}",
        ],
        ("code_block", true) => &[
            "// `{a}`: {w1} y luego {w2}\nlet valor = `{b}`; // conserva {w3}",
            "// maneja {w1} para `{a}`\nreturn `{b}`.aplicar({w2}); // {w3}",
            "// `{a}` envuelve {w1}
const resultado = `{b}`({w2}); // respaldo: {w3}",
            "// TODO: sacar {w1} de `{a}`
if (`{b}`) { {w2}(); } // {w3}",
        ],
        ("session_summary", false) => &[
            "Session summary: reviewed `{a}` and {w1}, adjusted {w2} around `{b}`, and left {w3} for later.",
            "We worked on {w1} in `{a}`, compared it with `{b}`, and agreed to keep {w2} and {w3} unchanged.",
            "Wrap-up: `{a}` got a pass over {w1}; `{b}` was compared against {w2} and {w3} moves to the next session.",
            "Today we revisited {w1} around `{a}`, noted that `{b}` still needs {w2}, and parked {w3}.",
        ],
        ("session_summary", true) => &[
            "Resumen de sesión: se revisó `{a}` y {w1}, se ajustó {w2} alrededor de `{b}` y se dejó {w3} para después.",
            "Trabajamos en {w1} dentro de `{a}`, lo comparamos con `{b}` y acordamos mantener {w2} y {w3} sin cambios.",
            "Cierre: `{a}` tuvo una revisión de {w1}; `{b}` se comparó con {w2} y {w3} pasa a la próxima sesión.",
            "Hoy retomamos {w1} alrededor de `{a}`, anotamos que `{b}` todavía necesita {w2} y dejamos {w3} en pausa.",
        ],
        ("user_preference", false) => &[
            "The user prefers {w1} over {w2} when working on `{a}`, and wants `{b}` to keep {w3}.",
            "Preference noted: for `{a}` use {w1}; for `{b}` avoid {w2} and {w3}.",
            "Standing preference: when touching `{a}`, favor {w1}; leave `{b}` alone unless {w2} and {w3} are involved.",
            "The user asked to keep {w1} simple in `{a}` and to mention `{b}` only when {w2} changes.",
        ],
        ("user_preference", true) => &[
            "El usuario prefiere {w1} antes que {w2} al trabajar en `{a}`, y quiere que `{b}` conserve {w3}.",
            "Preferencia anotada: para `{a}` usar {w1}; para `{b}` evitar {w2} y {w3}.",
            "Preferencia vigente: al tocar `{a}`, favorecer {w1}; dejar `{b}` como está salvo que intervengan {w2} y {w3}.",
            "El usuario pidió mantener {w1} simple en `{a}` y mencionar `{b}` solo cuando cambie {w2}.",
        ],
        ("architectural_decision", false) => &[
            "Decision: `{a}` will own {w1}; `{b}` keeps {w2}. The alternative of changing {w3} was discarded.",
            "We chose to route {w1} through `{a}` instead of `{b}`, accepting a cost in {w2} to protect {w3}.",
            "Trade-off recorded: `{a}` absorbs {w1} so that `{b}` can stay focused on {w2}; {w3} remains an open question.",
            "ADR note: we split {w1} between `{a}` and `{b}`, which makes {w2} easier to test and defers {w3}.",
        ],
        ("architectural_decision", true) => &[
            "Decisión: `{a}` se encarga de {w1}; `{b}` conserva {w2}. Se descartó la alternativa de cambiar {w3}.",
            "Elegimos canalizar {w1} por `{a}` en lugar de `{b}`, aceptando un costo en {w2} para proteger {w3}.",
            "Compromiso registrado: `{a}` absorbe {w1} para que `{b}` se concentre en {w2}; {w3} queda como pregunta abierta.",
            "Nota de ADR: repartimos {w1} entre `{a}` y `{b}`, lo que facilita probar {w2} y posterga {w3}.",
        ],
        (_, false) => &[
            "`{a}` relates to {w1} and {w2}; `{b}` is the place where {w3} is configured.",
            "In `{a}`, {w1} is derived from {w2}; `{b}` exposes {w3} to the rest of the system.",
            "`{a}` and `{b}` both touch {w1}; the open point is how {w2} interacts with {w3}.",
            "Notes on `{a}`: {w1} was reviewed, `{b}` still depends on {w2}, and {w3} is pending.",
        ],
        (_, true) => &[
            "`{a}` se relaciona con {w1} y {w2}; `{b}` es donde se configura {w3}.",
            "En `{a}`, {w1} se deriva de {w2}; `{b}` expone {w3} al resto del sistema.",
            "`{a}` y `{b}` tocan {w1}; el punto abierto es cómo interactúa {w2} con {w3}.",
            "Notas sobre `{a}`: se revisó {w1}, `{b}` todavía depende de {w2} y {w3} está pendiente.",
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

/// Sentences that mention an identifier while talking about something else,
/// so the row that holds the queried identifier does not read like the
/// question and a purely semantic match prefers its siblings.
pub fn mention_frames(kind: IdentKind, spanish: bool) -> &'static [&'static str] {
    match (kind, spanish) {
        (IdentKind::Function, false) => &[
            "While moving the importer behind the feature flag we left the call to `{id}` on the old path, so the nightly job still goes through it.",
            "The cleanup branch dropped the unused exports but kept `{id}` in place, since the report generator still imports it.",
        ],
        (IdentKind::Function, true) => &[
            "Al mover el importador detrás del feature flag dejamos la llamada a `{id}` en el camino viejo, así que el job nocturno sigue pasando por ahí.",
            "La rama de limpieza eliminó las exportaciones sin uso pero conservó `{id}`, porque el generador de reportes todavía lo importa.",
        ],
        (IdentKind::ErrorCode, false) => &[
            "During Tuesday's deploy the queue worker logged `{id}` twice; rolling back the schema change made it go away.",
            "Support forwarded a screenshot of the dashboard where `{id}` showed up next to the failed export, just before the weekend freeze.",
        ],
        (IdentKind::ErrorCode, true) => &[
            "En el deploy del martes el worker de la cola registró `{id}` dos veces; revertir el cambio de esquema lo hizo desaparecer.",
            "Soporte reenvió una captura del tablero donde `{id}` aparecía junto a la exportación fallida, justo antes del congelamiento del fin de semana.",
        ],
    }
}

/// A topic a related case asks about: the row that names the queried
/// identifier, three rows on the same topic that never name it, and the
/// question, which names both the identifier and the topic. Every text
/// names `{key}`; only `exact` and `query` name `{id}`.
pub struct RelatedTopic {
    pub exact_type: &'static str,
    pub exact: &'static str,
    pub related: [&'static str; 3],
    pub query: &'static str,
}

/// Two topics per kind of identifier and language.
pub fn related_topics(kind: IdentKind, spanish: bool) -> &'static [RelatedTopic; 2] {
    match (kind, spanish) {
        (IdentKind::Function, false) => &[
            RelatedTopic {
                exact_type: "semantic_fact",
                exact: "`{id}` is the entry point of the `{key}` nightly import; it parses each batch file before the ledger is touched.",
                related: [
                    "The `{key}` nightly import reads its batch files from the shared bucket after midnight.",
                    "When the `{key}` nightly import fails, the ledger keeps yesterday's totals until someone reruns it.",
                    "The `{key}` nightly import holds a lock on the ledger while a batch is written.",
                ],
                query: "how does {id} run the {key} nightly import",
            },
            RelatedTopic {
                exact_type: "semantic_fact",
                exact: "`{id}` warms the `{key}` cache on startup by loading the most requested entries first.",
                related: [
                    "The `{key}` cache is warmed on startup so the first requests do not miss.",
                    "Warming the `{key}` cache takes about two minutes and blocks the readiness probe.",
                    "If the `{key}` cache warmup is skipped, latency stays high until traffic fills it.",
                ],
                query: "what does {id} do when the {key} cache warms up",
            },
        ],
        (IdentKind::Function, true) => &[
            RelatedTopic {
                exact_type: "semantic_fact",
                exact: "`{id}` es el punto de entrada de la importación nocturna de `{key}`; procesa cada archivo del lote antes de tocar el libro mayor.",
                related: [
                    "La importación nocturna de `{key}` lee sus archivos de lote del bucket compartido después de medianoche.",
                    "Cuando falla la importación nocturna de `{key}`, el libro mayor conserva los totales de ayer hasta que alguien la vuelve a correr.",
                    "La importación nocturna de `{key}` bloquea el libro mayor mientras se escribe un lote.",
                ],
                query: "cómo corre {id} la importación nocturna de {key}",
            },
            RelatedTopic {
                exact_type: "semantic_fact",
                exact: "`{id}` precalienta la caché de `{key}` al arrancar, cargando primero las entradas más pedidas.",
                related: [
                    "La caché de `{key}` se precalienta al arrancar para que las primeras peticiones no fallen.",
                    "Precalentar la caché de `{key}` tarda unos dos minutos y bloquea la sonda de disponibilidad.",
                    "Si se omite el precalentamiento de la caché de `{key}`, la latencia sigue alta hasta que el tráfico la llena.",
                ],
                query: "qué hace {id} cuando se precalienta la caché de {key}",
            },
        ],
        (IdentKind::ErrorCode, false) => &[
            RelatedTopic {
                exact_type: "tool_error",
                exact: "`{id}` is what the `{key}` sync raises when the remote rejects a batch because of a schema mismatch.",
                related: [
                    "The `{key}` sync pushes local changes to the remote every five minutes.",
                    "A rejected batch stops the `{key}` sync until the schema on both sides matches again.",
                    "After a schema change, the `{key}` sync has to be restarted by hand.",
                ],
                query: "why does the {key} sync fail with {id}",
            },
            RelatedTopic {
                exact_type: "tool_error",
                exact: "`{id}` shows up when the `{key}` deploy cannot reach the registry to pull the new image.",
                related: [
                    "The `{key}` deploy pulls its image from the internal registry before draining the old pods.",
                    "When the registry is down, the `{key}` deploy stalls and the old version keeps serving.",
                    "The `{key}` deploy retries the image pull three times before giving up.",
                ],
                query: "what does {id} mean during the {key} deploy",
            },
        ],
        (IdentKind::ErrorCode, true) => &[
            RelatedTopic {
                exact_type: "tool_error",
                exact: "`{id}` es lo que lanza la sincronización de `{key}` cuando el remoto rechaza un lote por un esquema distinto.",
                related: [
                    "La sincronización de `{key}` envía los cambios locales al remoto cada cinco minutos.",
                    "Un lote rechazado detiene la sincronización de `{key}` hasta que el esquema vuelve a coincidir en los dos lados.",
                    "Después de un cambio de esquema, la sincronización de `{key}` hay que reiniciarla a mano.",
                ],
                query: "por qué falla la sincronización de {key} con {id}",
            },
            RelatedTopic {
                exact_type: "tool_error",
                exact: "`{id}` aparece cuando el despliegue de `{key}` no llega al registro para bajar la imagen nueva.",
                related: [
                    "El despliegue de `{key}` baja su imagen del registro interno antes de vaciar los pods viejos.",
                    "Cuando el registro está caído, el despliegue de `{key}` se traba y la versión vieja sigue atendiendo.",
                    "El despliegue de `{key}` reintenta bajar la imagen tres veces antes de rendirse.",
                ],
                query: "qué significa {id} durante el despliegue de {key}",
            },
        ],
    }
}

/// What can be said about an entity, one row per aspect, each with the
/// question that asks for exactly that aspect. Every row names `{key}`.
pub fn aspect_catalog(spanish: bool) -> &'static [(&'static str, &'static str); 8] {
    if spanish {
        &[
            ("El módulo `{key}` define la política de reintentos: tres intentos con backoff exponencial antes de rendirse.", "cuántas veces reintenta {key} una llamada fallida"),
            ("Las peticiones a `{key}` expiran a los 30 segundos, y ese presupuesto se comparte con quien llama.", "cuál es el timeout de las peticiones a {key}"),
            ("`{key}` se despliega los viernes por el pipeline de releases, nunca a mano.", "cuándo se despliega {key}"),
            ("Revertir `{key}` implica restaurar la etiqueta de imagen anterior y reaplicar el log de migraciones.", "cómo deshacemos un release malo de {key}"),
            ("Los operadores entran a `{key}` por el panel de administración, y cada acción queda en su log de auditoría.", "cómo acceden los operadores a {key}"),
            ("El equipo de pagos es dueño de `{key}` y atiende sus alertas en horario de oficina.", "qué equipo es responsable de {key}"),
            ("`{key}` guarda su estado en un archivo SQLite junto al binario, no en la base compartida.", "dónde persiste {key} su estado"),
            ("`{key}` exporta métricas de latencia y tasa de error al tablero cada minuto.", "qué telemetría publica {key}"),
        ]
    } else {
        &[
            ("The `{key}` module owns the retry policy: three attempts with exponential backoff before giving up.", "how many times does {key} retry a failed call"),
            ("Requests to `{key}` time out after 30 seconds, and that budget is shared with the caller.", "what is the request timeout of {key}"),
            ("`{key}` ships on Fridays through the release pipeline, never by hand.", "when does {key} get deployed"),
            ("Rolling back `{key}` means restoring the previous image tag and replaying the migration log.", "how do we undo a bad release of {key}"),
            ("Operators reach `{key}` through the admin panel, and every action lands in its audit log.", "how do operators get into {key}"),
            ("The payments team owns `{key}` and answers its pages during office hours.", "which team is responsible for {key}"),
            ("`{key}` keeps its state in a SQLite file next to the binary, not in the shared database.", "where does {key} persist its state"),
            ("`{key}` exports latency and error-rate metrics to the dashboard every minute.", "what telemetry does {key} publish"),
        ]
    }
}

/// Questions that need several aspects at once, as indices into
/// `aspect_catalog` plus the question text.
pub fn multi_aspect_queries(spanish: bool) -> &'static [(&'static [usize], &'static str)] {
    if spanish {
        &[
            (&[2, 3], "cómo se publica {key} y cómo se deshace un release"),
            (&[0, 1], "qué reglas de reintentos y timeout aplican a {key}"),
            (&[4, 5, 7], "quién opera {key}, cómo se entra y qué reporta"),
        ]
    } else {
        &[
            (&[2, 3], "how is {key} released and how is a release undone"),
            (&[0, 1], "what retry and timeout rules apply to {key}"),
            (&[4, 5, 7], "who runs {key}, how do people get in, and what does it report"),
        ]
    }
}

/// How a long memory opens: the only sentence that names `{key}`.
pub fn long_openings(spanish: bool) -> &'static [&'static str] {
    if spanish {
        &[
            "Estas son las notas de trabajo sobre `{key}`, el componente que está entre la cola de ingesta y la capa de almacenamiento.",
            "Resumen largo de lo que pasó con `{key}` durante el último trimestre, escrito para quien lo herede.",
            "Todo lo que conviene saber de `{key}` antes de tocarlo, juntado a partir de varias reuniones.",
        ]
    } else {
        &[
            "These are the working notes on `{key}`, the component that sits between the ingestion queue and the storage layer.",
            "A long write-up of what happened with `{key}` over the last quarter, kept for whoever inherits it.",
            "Everything worth knowing about `{key}` before changing it, gathered from several meetings.",
        ]
    }
}

/// One fact a long memory buries past its first piece: the sentence that
/// carries it, the answer as it appears in that sentence, and the question.
/// `{n}` is a number drawn per row; the question names `{key}`, the sentence
/// does not.
pub fn long_facts(spanish: bool) -> &'static [(&'static str, &'static str, &'static str); 6] {
    if spanish {
        &[
            ("Después de revisar el incidente, el presupuesto de reintentos quedó en {n} intentos por llamada.", "presupuesto de reintentos quedó en {n} intentos", "qué presupuesto de reintentos acordamos para {key}"),
            ("Al final el timeout de las peticiones quedó fijo en {n} segundos para todos los clientes.", "timeout de las peticiones quedó fijo en {n} segundos", "qué timeout fijamos para las peticiones de {key}"),
            ("El servicio pasó a escuchar en el puerto 70{n} detrás del proxy interno.", "escuchar en el puerto 70{n}", "en qué puerto escucha {key}"),
            ("Las entradas de la caché ahora vencen a los {n} minutos en lugar del valor anterior.", "vencen a los {n} minutos", "cuánto duran las entradas de la caché de {key}"),
            ("El congelamiento de despliegues dura {n} días al final de cada trimestre.", "congelamiento de despliegues dura {n} días", "cuánto dura el congelamiento de despliegues de {key}"),
            ("Los lotes quedaron limitados a {n} registros para que un volcado nunca trabe la cola.", "limitados a {n} registros", "cuál es el límite de registros por lote de {key}"),
        ]
    } else {
        &[
            ("After the incident review, the retry budget was set to {n} attempts per call.", "retry budget was set to {n} attempts", "what retry budget did we settle on for {key}"),
            ("In the end the request timeout was fixed at {n} seconds for every client.", "request timeout was fixed at {n} seconds", "what request timeout did we fix for {key}"),
            ("The service was moved to listen on port 70{n} behind the internal proxy.", "listen on port 70{n}", "which port does {key} listen on"),
            ("Cached entries now expire after {n} minutes instead of the old default.", "expire after {n} minutes", "how long do cached entries of {key} live"),
            ("The deploy freeze lasts {n} days at the end of each quarter.", "deploy freeze lasts {n} days", "how long is the deploy freeze for {key}"),
            ("Batches were capped at {n} records so a single flush never stalls the queue.", "capped at {n} records", "what is the batch size limit of {key}"),
        ]
    }
}

/// Sentences that pad a long memory between its opening and its fact. None
/// names the key or any fact of `long_facts`; `{n}` and `{w}` are drawn per
/// sentence, so two long rows never share a piece.
pub fn long_filler(spanish: bool) -> &'static [&'static str] {
    if spanish {
        &[
            "En la semana {n} el equipo repasó los tableros de {w} y no encontró nada que pidiera acción.",
            "Una reunión de seguimiento trató cómo se enrutan las alertas de {w} y quién las mira primero.",
            "Alguien propuso reescribir el módulo de {w}, pero la idea quedó en pausa hasta tener más datos.",
            "La prueba de carga corrió {n} horas y los gráficos de {w} se mantuvieron planos todo el tiempo.",
            "Casi toda la charla fue sobre nombres, documentación y cómo mantener al día las notas de {w}.",
            "Hubo un desvío corto hacia la actualización de la biblioteca de {w}, que se sigue en otro lado.",
            "Dos personas señalaron que el manual de {w} todavía describe la configuración del año pasado.",
            "El entorno de pruebas se reconstruyó {n} veces persiguiendo una advertencia de {w} sin relación.",
            "Nadie pudo reproducir el pico de {w} del martes anterior, así que el ticket se cerró.",
            "La rotación de guardias se ajustó para que las preguntas de {w} lleguen a alguien en menos de una hora.",
            "Se compartió un borrador del diagrama de {w}, con {n} cajas y demasiadas flechas.",
            "El grupo acordó revisar el presupuesto de {w} después del próximo ciclo de planificación.",
            "Los registros de los trabajos de {w} se recortaron para mantener controlada la factura de almacenamiento.",
            "Una pasante preguntó por qué las pruebas de {w} tardan tanto, y eso llevó a una hora útil de perfilado.",
            "El proveedor confirmó que el contrato de {w} se renueva solo en {n} semanas.",
            "Las dudas sobre la migración de {w} se juntaron en un documento compartido para más adelante.",
        ]
    } else {
        &[
            "During week {n} the team went through the {w} dashboards and found nothing that needed action.",
            "A follow-up meeting covered how the {w} alerts are routed and who looks at them first.",
            "Someone proposed rewriting the {w} module, but the idea was parked until there is more data.",
            "The load test ran for {n} hours and the {w} graphs stayed flat the whole time.",
            "Most of the discussion was about naming, documentation and how to keep the {w} notes current.",
            "There was a short detour into the {w} library upgrade, which is tracked somewhere else.",
            "Two people pointed out that the {w} runbook still describes the setup from last year.",
            "The staging environment was rebuilt {n} times while chasing an unrelated {w} warning.",
            "Nobody could reproduce the {w} spike from the previous Tuesday, so the ticket was closed.",
            "The on-call rotation was adjusted so that {w} questions reach someone within the hour.",
            "A draft diagram of the {w} flow was shared, with {n} boxes and far too many arrows.",
            "The group agreed to revisit the {w} budget after the next planning cycle.",
            "Logs from the {w} jobs were trimmed to keep the storage bill under control.",
            "An intern asked why the {w} tests take so long, which led to a useful hour of profiling.",
            "The vendor confirmed that the {w} contract renews automatically in {n} weeks.",
            "Questions about the {w} migration were collected in a shared document for later.",
        ]
    }
}

/// What a filler sentence is about.
pub fn long_topics(spanish: bool) -> &'static [&'static str] {
    if spanish {
        &["latencia", "facturación", "registros", "búsqueda", "incorporación", "planificador", "métricas", "respaldos", "red", "seguridad", "reportes", "colas"]
    } else {
        &["latency", "billing", "logging", "search", "onboarding", "scheduler", "metrics", "backup", "network", "security", "reporting", "queueing"]
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
                assert!(frames.len() >= 4, "{t} {spanish}");
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
    fn mention_frames_name_the_identifier_in_a_frame_no_lexical_row_uses() {
        for kind in [IdentKind::Function, IdentKind::ErrorCode] {
            for spanish in [false, true] {
                let mentions = mention_frames(kind, spanish);
                assert!(mentions.len() >= 2, "{kind:?} {spanish}");
                for mention in mentions {
                    assert_eq!(mention.matches("{id}").count(), 1, "{mention}");
                    assert!(mention.contains("`{id}`"), "{mention}");
                    assert!(
                        lexical_frames(kind, spanish).iter().all(|f| f.content != *mention),
                        "{mention}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_related_topic_names_the_identifier_only_in_its_exact_row_and_query() {
        for kind in [IdentKind::Function, IdentKind::ErrorCode] {
            for spanish in [false, true] {
                for topic in related_topics(kind, spanish) {
                    assert_eq!(topic.exact.matches("`{id}`").count(), 1, "{}", topic.exact);
                    assert_eq!(topic.query.matches("{id}").count(), 1, "{}", topic.query);
                    for text in [topic.exact, topic.query].iter().chain(&topic.related) {
                        assert!(text.contains("{key}"), "{text}");
                    }
                    assert!(topic.related.iter().all(|r| !r.contains("{id}")));
                }
            }
        }
    }

    #[test]
    fn the_aspect_catalog_has_eight_distinct_aspects_in_each_language() {
        for spanish in [false, true] {
            let catalog = aspect_catalog(spanish);
            assert_eq!(catalog.len(), 8);
            for (row, question) in catalog {
                assert!(row.contains("`{key}`"), "{row}");
                assert!(question.contains("{key}"), "{question}");
            }
            let rows: std::collections::HashSet<_> = catalog.iter().map(|(r, _)| r).collect();
            assert_eq!(rows.len(), 8);
        }
    }

    #[test]
    fn multi_aspect_queries_ask_for_two_or_three_distinct_aspects_of_the_catalog() {
        for spanish in [false, true] {
            let queries = multi_aspect_queries(spanish);
            assert!(queries.len() >= 2);
            for (aspects, query) in queries {
                assert!((2..=3).contains(&aspects.len()), "{query}");
                assert!(aspects.iter().all(|&a| a < aspect_catalog(spanish).len()));
                let unique: std::collections::HashSet<_> = aspects.iter().collect();
                assert_eq!(unique.len(), aspects.len());
                assert!(query.contains("{key}"), "{query}");
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
