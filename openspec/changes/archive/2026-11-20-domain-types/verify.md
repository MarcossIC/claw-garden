# domain-types — Verificación

**Resultado:** la tanda 1 aplicada y verificada en Linux, en el CI de Linux y Windows y en Windows en local ([«Windows»](#windows)). La suite entera y los dos clippy pasan sobre el último commit de código, y ni `crates/clawg-cli/tests/api.rs`, ni los dos contratos del agente, ni las migraciones cambian. Salió en 0.90.0.

## Qué se corrió

Sobre el último commit de código (`test(conventions): check the statuses and types the SQL compares`), en Linux, con un único directorio de build (`target`):

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo; incluye los doctests |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos; es el núcleo sin `testing` ni `eval`, como lo construye el release |

La suite de `develop` antes del cambio también pasaba en la misma máquina.

## Los tests en rojo

| Test | Qué se vio antes del código |
|---|---|
| Los ocho de `Stored<T>` y `MemoryStatus` en `crates/clawg-core/src/memory/domain/record.rs` (R1, R3, R4) | no compilaban: `MemoryStatus`, `StoredType` y `StoredStatus` no existían |
| `a_row_of_a_type_the_code_does_not_know_is_still_found`, `rows_of_a_type_or_status_the_code_does_not_know_stay_listed`, `an_event_of_a_type_the_code_does_not_know_parses_and_keeps_it` (R2) | no compilaban: comparan con `StoredType::Other` y `known()` |
| `a_status_the_code_does_not_know_never_ranks`, `the_tier_udf_keeps_the_tier_of_a_type_it_does_not_know`, el caso `archived` de `source_state_cases` (R4) | no compilaban: los fixtures construyen `StoredStatus` y `compute_tier` recibía `&str` |
| `compared_names_reads_the_literals_a_line_compares` (R6) | no compilaba: `compared_names` no existía |

## R6 detecta un literal mal escrito

Con `status = 'superseded'` cambiado a mano por `'superseeded'` en `REVIVE_SQL` (`crates/clawg-core/src/memory/revision/dispute.rs`), `every_status_and_type_the_sql_compares_is_one_the_code_knows` falla y nombra `clawg-core/src/memory/revision/dispute.rs:347: status "superseeded"`. Con el literal de vuelta, pasa. El cambio no se commiteó.

## Lo que ve un cliente

`git diff d2416b5 -- crates/clawg-cli/tests/api.rs crates/clawg-cli/src/mcp/agent_contract.json crates/clawg-cli/src/mcp/agent_contract.es.json crates/clawg-core/migrations` no muestra nada. Los dos tests de contrato pasan sin `CLAWG_BLESS`.

## Lo que no se midió

- **`rag-eval --diff`:** el golden set de `docs/eval/` está fuera de git y no estaba en el clon donde se verificó. El ranking no cambia por construcción: `lambda_for` y la puerta de estado devuelven, para cada nombre, el valor que devolvía el literal, y los tests de `score.rs` lo fijan, incluido el nombre desconocido.
- **`perf-bench`:** no se corrió. Un `Stored<T>` conocido es un enum `Copy` sin asignación; uno desconocido guarda el mismo `String` que se leía antes.

## Windows

- CI de Linux y Windows: workflow run 44591210821 sobre `d658c8a9` (head del PR #155, que publicó 0.90.0), jobs `test (ubuntu-latest)` y `test (windows-latest)`. `d658c8a9` contiene los cuatro commits de código: `01b4c546`, `7878784d`, `6ac1108f` y `fe74b010`.
- Las dos correcciones, `6ac1108f` (`fix(core): read a known name held in Stored::Other as its variant`) y `fe74b010` (`test(conventions): follow a compared SQL name onto the lines after it`), llegaron después de la corrida de Linux de arriba. Las cubren el CI y la corrida local de Windows.
- Local, en Windows, sobre `fbb8d91f` y después de `cargo clean --workspace --target-dir target-test`: `cargo test --workspace --target-dir target-test` pasa sin ningún fallo, y `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings` y `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings` no tienen hallazgos.

## Pendiente

Nada bloquea el archivo; lo que no se midió está en [«Lo que no se midió»](#lo-que-no-se-midió). Las tandas 2 a 5 son cambios propios, en [«Tandas siguientes»](tasks.md#tandas-siguientes).
