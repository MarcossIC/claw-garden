# context-hit-room — Verificación

## Linux (2026-10-22)

Sobre `7053955`, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.

## RED antes del cambio

Con el campo `hit_room` agregado y sin conducta (todo contexto con reserva 0, el orden de 23):

| Requisito | Cómo fallaba |
|---|---|
| R1 | `a_mature_store_still_shows_the_hit_of_the_query` no encontró el hit de la consulta en RECENT ni en LONG-TERM: el recorte sacó los once hits y dejó el workspace, el resumen recortado y el par, el caso que se vio en un `recall_context` real. `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts` dejó el resumen y cortó los dos mejores hits. `the_hits_over_their_room_go_before_the_last_session` y `a_room_never_takes_the_prompt_over_the_budget` ya pasaban: el primero guarda que lo que pasa de la reserva sigue cortándose primero, el segundo el tope del presupuesto con cualquier reserva |
| R2 | `a_lone_hit_over_its_room_is_shortened_to_it` acortó el hit hasta que entró el resumen entero, en lugar de dejarle la reserva y acortar el resumen |
| R3 | `a_context_without_a_query_keeps_the_last_session_over_its_saves` ya pasaba: guarda que el contexto sin consulta conserve el orden de 23 |

Los tests de 23 corren con reserva 0 y pasan sin cambios.

## Lo que no se midió

- El `recall_context` real que cortaba los 10 hits y `revision-eval chains --real --background` piden el modelo real y el store de prod o un export suyo, con autorización del dueño del store. Quedan pendientes en las [tareas](tasks.md); la regla se comprobó con un store de la misma forma armado en el test.
- `rag-eval --real --agent-view`: con el default ningún prompt del golden se corta (23), así que el orden de corte no lo mueve.
- Windows no se corrió a mano: el cambio no toca código que dependa de la plataforma, y el CI del pull request cubre la suite y clippy allí.
