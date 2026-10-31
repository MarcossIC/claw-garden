# native-memory-import — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada grupo es un commit convencional atómico, que se comprueba con las pruebas de los archivos que cambia. La suite completa y los dos clippy de `.claude/CLAUDE.md` se corren una vez, sobre el último commit del cambio. Los fixtures se arman a mano; ningún test necesita el modelo real, la red ni un perfil de datos, y nada queda en el temporal. Los commits se arman con rutas explícitas, sin referencia a planes, fases ni cambios, y sin atribución.

Requisitos en la [spec](spec.md) (NM1 a NM7) y decisiones en el [diseño](design.md) (D1 a D6).

## 1. El adaptador (NM1 a NM5; D1, D2, D5)

- [ ] RED en `crates/clawg-core/src/memory/portability/claude_code.rs`: los tests de NM1 a NM3 y NM5. Fallan por no compilar.
- [ ] GREEN: `MemoryDocument`, `read` (texto), `from_document`, `collect` (disco), el lector de frontmatter y el del índice, `origin` e `is_topic_file`.
- [ ] RED en `translate.rs`: `claude_code_types_are_preferences_or_facts`. GREEN: `memory_type_of` por fuente.
- Commit: `feat(import): read Claude Code's auto memory into foreign notes`

## 2. El import (NM6; D1, D5, D6)

- [ ] RED: `parse_input` y `read_export` con `claude-code` en `crates/clawg-cli/src/daemon/api.rs`; `import_takes_a_source_and_project_maps` con `--from claude-code`, un nombre con guion inicial y `--map-root` refusado en `cli.rs`; el informe sin versión en `import.rs`; `the_claude_code_document_imports_by_text` en `tests/api.rs`; `a_claude_code_memory_imports_through_the_binary` en `tests/binary.rs`.
- [ ] GREEN: `Source::ClaudeCode`, la lectura del directorio en el job, `ImportSource::ClaudeCode`, `allow_hyphen_values` en `--project-map`, el informe.
- Commit: `feat(import): import Claude Code's auto memory with --from claude-code`

## 3. El aviso de `doctor` (NM7; D4)

- [ ] RED: `the_facts_name_the_origins_imported_from_claude_code` en `inspect.rs`, `native_memory_is_named_until_every_file_is_imported` en `doctor/checks.rs`, `doctor_names_native_memory_not_imported` en `tests/binary.rs`.
- [ ] GREEN: `StoreFacts::claude_code_imports`, `checks::native_memory`, `mcp::registered`, la lista de archivos de tema en `doctor::run`.
- Commit: `feat(doctor): name Claude Code's auto memory that is not imported`

## 4. La referencia

- [ ] `docs/reference/commands.md` (`clawg import --from claude-code`, el hallazgo de `doctor`) y `docs/reference/daemon.md` (`from: "claude-code"`).
- Commit: `docs: describe the import of Claude Code's auto memory`

## 5. Verificación

- [ ] Suite completa y los dos clippy sobre el último commit; `verify.md`.
- Commit: `docs(openspec): record the verification of native-memory-import`

## 6. Archivo

- [ ] Los requisitos en `specs/portability` y `specs/doctor`; el cambio a `changes/archive/`; el mapa de `openspec/README.md`.
- Commit: `docs(openspec): archive native-memory-import and fold it into the specs`
- [ ] Subir el minor, `cargo update --workspace`. Commit: `chore: bump version to 0.72.0`
