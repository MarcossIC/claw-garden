# daemon-hardening — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El token y `daemon.json` nacen en 0600 (R5)

- [x] Tests primero en `lifecycle.rs`: `a_created_file_is_owner_only_before_anything_is_written`, `an_old_world_readable_token_is_replaced_by_an_owner_only_file`, `state_file_is_owner_only_on_unix`.
- [x] `create_owner_only` (borra el anterior y crea con `create_new` y modo 0600); `write_token` y `write_state` la usan; `restrict_to_owner` se va.
- Commit: `fix(cli): create the token and daemon.json owner-only from the start`

## 2. La decisión pura y el registro con tope (R1, R4)

- [x] Tests primero en `daemon/guard.rs`: la tabla de R1 y el tope del registro.
- [x] `admits(host, uri_authority, origin)`, `RefusalLog`.
- Commit: con la tarea 3

## 3. El guardia en `router` (R2, R3)

- [x] Tests primero en `tests/api.rs`: `a_foreign_host_is_forbidden_on_health_and_v1_with_or_without_a_token`, `an_origin_is_forbidden_even_with_the_token`, `both_refusals_answer_the_same_body`, `a_daemon_client_passes_the_guard`.
- [x] La capa en `router`, antes del token (`admit_loopback`).
- Commit: `feat(cli): refuse a non-loopback Host and any Origin on every daemon route`

## 4. Docs

- [ ] `.claude/CLAUDE.md`, README, `openspec/README.md`, `state.yaml`.
- Commit: `docs: say the daemon refuses a foreign Host and any Origin`

## 5. Verificación

- [ ] Suite y clippy en Linux sobre el último commit.
- [ ] Un daemon real del perfil dev en un home de prueba, con `curl`: `Host` ajeno y `Origin` dan 403; `clawg status` y `clawg stop` funcionan.
- [ ] `verify.md`.
