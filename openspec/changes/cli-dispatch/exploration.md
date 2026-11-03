# cli-dispatch — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — | exploración (borrador) |

**Tesis:** `dispatch` es una sola función de 445 líneas con 31 comandos, y sus errores son `String`. La misma ceremonia se repite en cada brazo, y la lógica de cada comando no se puede probar sin pasar por `main`.

## Problema (verificado contra el código)

En `crates/clawg-cli/src/main.rs`:

- `async fn dispatch(cli: Cli) -> Result<ExitCode, String>` (`:123`) ocupa 445 líneas, con 31 brazos `Command::`.
- `.map_err(|e| e.to_string())` aparece 66 veces y `profile.resolve()` 27.
- Al pasar a `String` se pierde el tipo del error, así que el código de salida no puede depender de qué falló sin volver a mirar el comando.
- El brazo de `Status` (`:146-172`) hace trabajo real inline: lee el estado, consulta `/health`, sondea el store y arma el reporte.

`crates/clawg-cli/src/cli.rs` ya separa el parseo como lógica pura, y hay comandos con su módulo (`projects.rs`, `import.rs`, `doctor/`). El patrón existe; `dispatch` es lo que quedó sin aplicarlo.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Una función por comando en su módulo, con `CliError` tipado y la resolución del perfil en un solo lugar** | `dispatch` queda como una tabla. Cada comando se prueba solo | Muchos commits chicos |
| B. Sólo un helper para perfil y directorio de datos | Saca 50 líneas enseguida | La función sigue siendo enorme y los errores siguen siendo `String` |
| C. Un trait `Command` con `run` y registro de comandos | Uniforme | Una abstracción sin variación real: un `match` ya lo resuelve. Rechazada |

**Recomendado: A.** Un `match` que delega es exactamente lo que `clap` espera. No hace falta un trait.

## Alcance candidato

1. `CliError` con `thiserror`, y `From` para los errores que ya existen.
2. Mover cada brazo largo a una función de su módulo.
3. Que `dispatch` sólo resuelva lo común y delegue.

### No-alcance

- Cambiar comandos, argumentos, mensajes, salida o códigos de salida.
- `serve::run` (232 líneas, `crates/clawg-cli/src/daemon/serve.rs:105`), que tiene otro problema: es el arranque del daemon.

## Decisiones abiertas

1. Dónde se traduce un error al idioma del usuario: hoy algunos brazos arman el texto en inglés y en español inline (`main.rs:182-187`). `restore.rs` y `secrets.rs` ya tienen una función `explain`, que es un buen precedente.

## Cómo se mide

- `crates/clawg-cli/tests/binary.rs`, que corre el binario real, pasa sin editarse. Ese es el contrato de mensajes y códigos.
- **Test en rojo:** un comando con lógica, como `status`, se prueba sin pasar por `main`.

## Interacciones

- **clippy-enforced:** `dispatch` es candidato a `too_many_lines` si ese lint se activa.
- Puede correr en paralelo con todo lo demás: sólo toca `clawg-cli` y no depende del almacenamiento.
