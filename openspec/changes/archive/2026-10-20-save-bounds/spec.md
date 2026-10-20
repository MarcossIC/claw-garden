# save-bounds — Especificación

## R1. El contenido de un save tiene un tope en bytes

`POST /v1/memories` MUST rechazar con `400` un `content` de más de `MAX_CONTENT_BYTES` (65 536) bytes en UTF-8, contados como llegan, antes de enmascarar. El rechazo MUST ocurrir donde hoy se rechaza el contenido vacío: antes de validar el tipo, la clave y el `operation_id`, antes de consultar el estado del embedder y antes de escribir la fila del outbox o encolar el evento. El mensaje MUST nombrar el campo, el tamaño recibido y el tope, y decir que el texto se parte en varios saves:

```
content is 70000 bytes; at most 65536 are accepted: split it into several saves, one fact each
```

Un `content` de exactamente `MAX_CONTENT_BYTES` bytes MUST aceptarse como cualquier otro. La decisión es una función pura (`check_content`) que también rechaza el contenido vacío o de sólo espacios con `content must not be empty`, como hoy.

Una fila del outbox escrita antes del tope, de cualquier tamaño, MUST seguir guardándose al reproducirse: el tope está en el handler y no en el worker.

#### Escenario: sobre el tope
- GIVEN un daemon sin worker
- WHEN se guarda un `content` de 65 537 bytes
- THEN responde `400`, el mensaje nombra `65537` y `65536`
- AND el outbox queda vacío

#### Escenario: en el tope
- GIVEN un daemon con el embedder listo
- WHEN se guarda un `content` de 65 536 bytes
- THEN el save se acepta y sus pedazos quedan guardados

#### Escenario: bytes, no caracteres
- GIVEN un texto de 32 769 `ñ` (65 538 bytes)
- WHEN `check_content` lo evalúa
- THEN lo rechaza; uno de 32 768 `ñ` (65 536 bytes) pasa

**Tests:** `content_is_checked_in_bytes` (`api.rs`, mod tests); `a_save_over_the_content_limit_is_refused_before_the_outbox` y `a_save_at_the_content_limit_is_stored` (`crates/clawg-cli/tests/api.rs`).

## R2. El agente conoce el tope

La descripción del argumento `content` de `save_memory` MUST nombrar el tope (64 KiB) y `protocol.md` MUST decirlo en «How to write a save», fuera de la apertura que limita la prueba de 2 048 caracteres. Un save rechazado por el tope MUST llegar al modelo como `not saved: ` seguido de la frase del daemon, como cualquier `400`.

#### Escenario: el puente
- GIVEN un daemon
- WHEN el puente guarda 65 537 bytes
- THEN el resultado es un error que empieza con `not saved: content is 65537 bytes`

**Tests:** `save_memory_says_a_save_over_the_limit_was_not_saved` (`server.rs`, mod tests); `the_agent_contract_matches_its_file`, después de bendecir.
