# protected-verdicts — Diseño

## D1. La guarda mira la fila que pierde, en todo estado

**Elegido:** en `judge_pair`, después de `check_pair`, `supersedes` con `other_row.protected()` y sin declaración es `NeedsUser`, sin mirar `state`. La declaración se valida antes de abrir la transacción (`AnswerRequired`), como ya se hace con `ReasonRequired`.

**Alternativas:** mirar los dos lados en todo estado, como hoy en `pending` (bloquearía deshacer, que pone la protegida en `keep`); guardar sólo `escalated` y `judged` (deja `pending` y el par sin fila); prohibir `conflicts` sobre una protegida (es justo el veredicto correcto cuando sólo el usuario sabe).

**Efecto en `pending`:** hoy un hecho no puede reemplazarse por una decisión en un par `pending` sin pasar por `conflicts`; con D1 sí, porque la que pierde no es protegida. Es la decisión «sólo la que pierde».

**Ruta vieja:** `POST /v1/disputes/resolve` (`resolve_dispute_core`) no tiene dónde declarar; pasa `false`, así que un puente anterior nunca reemplaza una protegida. Es el lado seguro.

## D2. El registro en `reason`, marcado

**Elegido:** sin migración, el `reason` del par lleva `user answered: <reason enmascarado>` cuando el veredicto trae la declaración, sea cual sea el veredicto. El prefijo deja leer la historia sin una columna, y la marca va después de `redact` para no depender de él.

**Alternativas:** una columna `judged_by` (V18, coordinada con 51, 54 y 56; sólo si `memory_status` o la exportación necesitan leerlo sin texto); guardar el `reason` tal cual (no distingue el declarado).

## D3. El descenso de tipo

**Elegido:** `MemoryPatch` gana `user_answered` (un método `answered`, para no cambiar la firma de `new`, que valida lo que no depende de la fila). `update_memory_core` lee el tipo actual dentro de su transacción y, si es protegido y el nuevo no, devuelve `UpdateError::NeedsUser { id, from }`. Para eso deja de devolver `MemoryError` y devuelve `UpdateError` (con `Store(#[from] MemoryError)`), como `judge_pair` devuelve `ResolveError`. El daemon lo traduce a `409`.

`MemoryType::is_protected` es la única lista de tipos protegidos; `PairRow::protected` la usa a través del parser del tipo.

**Alternativa descartada:** rechazar el descenso desde la herramienta (ii): hoy no hay otra vía para el usuario.

## D4. Compatibilidad del cable

- `JudgeRequest` y `UpdateRequest` toman `user_answered` con `#[serde(default)]`. `UpdateRequest` sigue con `deny_unknown_fields`.
- El cliente manda `user_answered` en `judge` siempre (el `JudgeRequest` anterior ignora campos desconocidos) y en `update` sólo cuando es `true` (`skip_serializing_if`), así un puente nuevo con un daemon anterior sigue cambiando importancia y tipo como antes; con la declaración, un daemon anterior responde `400` nombrando el campo, y un daemon anterior no tiene la guarda.
