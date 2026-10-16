# quality-guards — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas que se descartaron. Las alternativas más amplias (CI, `selftest`, latencia, generar docs desde el código) están en la [exploración](exploration.md#descartado).

## D1. Dónde vive el archivo del contrato

**Elegido:** `crates/clawg-cli/src/mcp/agent_contract.json`, junto a `server.rs`. El test lo lee en tiempo de ejecución desde `concat!(env!("CARGO_MANIFEST_DIR"), "/src/mcp/agent_contract.json")`.

| Alternativa | Por qué no |
|---|---|
| `crates/clawg-cli/tests/` | El test necesita `tool_router` y `get_info`, que se arman en el `mod tests` de `server.rs` (`bridge`, `server.rs:1053`). Un test de integración tendría que hablar por stdio o abrir visibilidad sólo para esto |
| `docs/` | Queda lejos del código que lo produce y fuera del crate; el diff se revisa mejor al lado de `server.rs` |
| `include_str!` | Compila el archivo dentro del binario de test; bendecir obliga a leer y escribir la misma ruta igual, así que no ahorra nada |

## D2. Qué entra

**Elegido:** cada `rmcp::model::Tool` serializado entero, como lo manda `tools/list`, más el texto de `instructions`. Hoy eso es `name`, `description` e `inputSchema` (las herramientas no declaran `title`, `outputSchema`, `annotations`, `icons` ni `_meta`), pero si un cambio agrega uno de ésos, también aparece en el diff.

No entra `serverInfo`: el nombre depende del perfil (`Profile::mcp_server_name`) y la versión es `CARGO_PKG_VERSION`, que cambia en cada bump (`server.rs:1017`).

| Alternativa | Por qué no |
|---|---|
| Sólo `name`, `description` e `inputSchema` | Un campo nuevo que el cliente sí recibe (por ejemplo `annotations.readOnlyHint`) quedaría sin fijar |
| La respuesta entera de `tools/list` por stdio | Arrastra el sobre JSON-RPC, `_meta` de paginación y, en el ciclo 2026-07-28, `ttlMs` y `cacheScope`, que no deciden qué llama el modelo |

## D3. Comparar y escribir

El contrato es un `serde_json::Value` de esta forma:

```json
{
  "instructions": "...",
  "tools": { "delete_memory": { "description": "...", "inputSchema": { } , "name": "delete_memory" } }
}
```

- **Comparar** es una función pura `contract_diff(expected: &Value, actual: &Value) -> Vec<String>` que devuelve una línea por herramienta distinta, sobrante o faltante, y una por las instrucciones. Indexar las herramientas por nombre hace que el orden de registro no importe.
- **El orden de claves.** `serde_json` está compilado con `preserve_order` en este workspace (`cargo tree -e features -i serde_json`, lo activa una dependencia), así que un `Map` guarda el orden de inserción. La igualdad de `Value` no depende de ese orden (la de `IndexMap` compara pares), y el test puro `contract_diff_ignores_key_order` lo fija para no depender de eso sin saberlo.
- **Escribir** es `render_contract(&Value) -> String`: ordena las claves de todos los objetos de forma recursiva, `to_string_pretty` y un `\n` final. Así el archivo no cambia de bytes entre dos bendiciones, aunque `schemars` reordene claves.

## D4. Cómo se bendice

**Elegido:** `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`. Con la variable, el test escribe `render_contract(actual)` y **falla** con «agent contract updated: review the diff of agent_contract.json and run the tests again». Si el archivo no existe, el test falla diciendo cómo crearlo.

| Alternativa | Por qué no |
|---|---|
| La variable reescribe y pasa | Una variable olvidada en el shell bendice todo en silencio y la suite queda verde |
| Un test `#[ignore]` que regenera | Dos tests para lo mismo, y `cargo test -- --ignored` corre también los demás ignorados del workspace |
| Un binario o `cargo run` auxiliar | Superficie nueva en el crate por algo que el test ya sabe hacer |

## D5. Inventario de `.claude/CLAUDE.md`

`tool_names_in_doc(doc: &str) -> Option<BTreeSet<String>>`: toma la primera línea que empieza con `MCP tools (`, se queda con lo que sigue al primer `): ` y junta cada texto entre comillas invertidas. Devuelve `None` si la línea no existe, y el test falla con eso en lugar de pasar con un conjunto vacío.

El test lee `CARGO_MANIFEST_DIR/../../.claude/CLAUDE.md`. Funciona porque los tests corren desde el repo; los crates no se publican (ningún `Cargo.toml` declara `publish`, y ninguno se empaqueta), así que no hay tarball donde falte el archivo.

`AGENTS.md` pasa a ser un puntero (tarjeta del hilo) y no se lee. La spec `mcp` tampoco: sus nombres van dentro de una oración (`openspec/specs/mcp/spec.md:59`).

## D6. Compuerta de código muerto

**Elegido:** un test de integración, `crates/clawg-cli/tests/conventions.rs`, que recorre con `std::fs` los `.rs` de `crates/*/src` y `crates/*/tests` (raíz: `CARGO_MANIFEST_DIR/../..`) y reporta `ruta:línea` de cada atributo que esconde código muerto.

`hides_dead_code(line: &str) -> bool` es pura: la línea, sin espacios al principio, empieza con `#[` o `#![`; el atributo es `allow`, `expect` o un `cfg_attr` que los contiene; y entre sus lints aparece `dead_code`, `unused` o un nombre que empieza con `unused_`. Una línea de comentario o un literal no empiezan con `#[`, así que no cuentan; los fixtures del propio test van dentro de strings por la misma razón.

| Alternativa | Por qué no |
|---|---|
| `[workspace.lints.rust] dead_code = "deny"` | Rompe el paso RED de TDD: una función recién escrita sin llamador no compila |
| `#![deny(dead_code)]` en cada raíz | Igual que la anterior |
| Sólo `clippy -D warnings` documentado | Sin CI depende de que alguien lo corra; el test corre en cada `cargo test` |
| Un hook de git | No se clona con el repo |

El test no ve un `dead_code` que el compilador marca sin `allow`; eso lo cubre `clippy` (D7) y R5 deja la suite sin warnings.

## D7. `clippy` documentado

**Medido en `main` (`638812b`, Linux):** `cargo clippy --workspace --all-targets -- -D warnings` no pasa. Sin `-D`, da 30 hallazgos: 29 de `clippy` (7 `too_many_arguments`, de los que sólo 2 tienen `allow`; 5 de eficiencia; 4 implementaciones manuales; 3 tipos complejos; 2 líneas vacías después de un doc comment; un `transmute` sin anotar; un ítem más nuevo que el MSRV, y otros sueltos) y uno de `rustc`, `insert_row_with_embedding`. Arreglarlos toca firmas de funciones en varios crates y no es de este cambio.

**Elegido:** el comando que va a `.claude/CLAUDE.md` (Testing) hace fallar sólo los warnings del compilador, que son los que incluyen `dead_code` y `unused`:

```
cargo clippy --workspace --all-targets -- -A clippy::all -D warnings
```

En Windows, con `--target-dir` aparte por el lock de `clawg.exe`. Después de R5 termina con código 0 (`verify.md`). Los hallazgos de `clippy` que quedan (28: uno estaba en código que este cambio borra) van al backlog del README de openspec; cuando se arreglen, el comando pierde `-A clippy::all`.

| Alternativa | Por qué no |
|---|---|
| Arreglarlos acá | Cambia firmas (`too_many_arguments`) y código de varios módulos en un cambio de redes de tests |
| `RUSTFLAGS="-D warnings" cargo test` | Cambiar `RUSTFLAGS` recompila todas las dependencias y descarta el caché de la suite normal |
| Documentar `-D warnings` sin más | Un comando documentado que falla hoy se deja de correr |
