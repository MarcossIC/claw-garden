# cli-dispatch — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); verificado otra vez contra `develop` (`a7f7337`), en `crates/clawg-cli/src/main.rs`:

- `async fn dispatch(cli: Cli) -> Result<ExitCode, String>` (`:123`) ocupa 445 líneas (`:123-567`), con 31 brazos `Command::`.
- `.map_err(|e| e.to_string())` aparece 66 veces y `profile.resolve()` 27. Al pasar a `String` se pierde el tipo del error.
- 13 brazos (`Import`, `Undelete`, `Outbox`, los 9 de `Projects` y `Memories`) repiten las mismas líneas: el directorio de datos, el idioma, `probe_running(&base_dir, false)` y `memories::explain_unavailable` para decir que no hay daemon (por ejemplo `:388-395`).
- Hay brazos que hacen trabajo propio y sólo se prueban corriendo el binario: `Status` (`:146-172`) lee el estado, consulta `/health`, sondea el store, arma el informe y decide el código de salida; `Backup` (`:196-211`) elige entre el daemon y la copia sin daemon; `Export` (`:222-235`) resuelve el proyecto; `Import` (`:237-270`) elige entre importar y deshacer; `Purge` (`:291-310`) entre una memoria y un borrado; `Metrics` (`:375-385`) entre el informe y los eventos.
- `Stop` compone su negativa en inglés y en español dentro del brazo (`:182-187`), mientras que `restore` y `secrets` la componen con una función `explain` de su módulo (`crates/clawg-cli/src/restore.rs:23`, `crates/clawg-cli/src/secrets.rs:342`), probada en los dos idiomas.
- `run_path` (`:569-612`), lo que hace `clawg path`, vive en `main.rs`, aunque la documentación del archivo (`:3-5`) dice que lo que hace cada subcomando vive en `clawg_cli`.

## Objetivo que sirve

Transversal. No cambia conducta: deja cada comando con lógica probado sin `main`, y el error tipado hasta el borde.

## Decisiones

1. **Enfoque A de la exploración**: una función por comando en su módulo, un `CliError` tipado con `thiserror` y lo común resuelto una vez. Sin un trait `Command` (C, rechazado en la exploración): un `match` que delega es lo que `clap` espera, y un trait no tendría variación que abstraer.

2. **Dónde se traduce un error al idioma del usuario (la decisión abierta de la exploración): en el módulo del comando, cuando falla, con el idioma que el comando ya leyó.** La frase la compone una función pura del módulo, `explain(lang, err, …) -> String`, al lado de sus informes y probada en los dos idiomas, como ya hacen `restore::explain`, `secrets::explain`, `memories::explain_unavailable` (`crates/clawg-cli/src/memories.rs:362`) y los `*_refused` de `projects.rs`. Las alternativas:
   - **En `CliError`**, con variantes que lleven el `Lang` y el error y un `Display` que traduzca. Juntaría en un archivo frases de todos los comandos (cómo se libera el store no se dice igual para restaurar que para purgar), metería el idioma dentro de un valor de error y haría que el `Display` de un error tipado dependiera de una preferencia, cuando cli-language decidió que la causa técnica se muestra tal cual, en inglés, dentro de la frase que la enmarca ([decisión 4 de su propuesta](../2026-10-23-cli-language/proposal.md#decisiones)).
   - **En `main`, al imprimir**: `main` no tiene el directorio de datos para leer el idioma ni sabe qué estaba haciendo el comando cuando falló.

   Lo que se sigue de eso:
   - `CliError` tiene una variante por cada error tipado que hoy se muestra con `e.to_string()`, con `#[error(transparent)]` y `#[from]`: su texto es el `Display` de la fuente, el mismo de hoy, y `?` reemplaza a `.map_err(|e| e.to_string())`.
   - Y una variante `Failed(String)` para la frase que clawg ya compuso en el idioma del comando: lo que devuelve un `explain`, la negativa de `purge` sin `--yes`, o los errores en `String` que ya devuelven `sharing::run`, `preferences::run`, `metrics` y `export::run`. Es el mismo molde que `ImportError::Failed` (`crates/clawg-cli/src/import.rs:31`) y `UpdateError::Failed` (`crates/clawg-cli/src/update/mod.rs:65`). No hay `From<String>`: una frase entra explícita, con `CliError::Failed`, así un error tipado no vuelve a texto sin que se vea.
   - `main` sólo imprime `clawg: {error}` en stderr y sale con 1, como hoy; ni `main` ni `dispatch` componen frases.
   - La negativa de `Stop` pasa a `stop::explain`, con el mismo texto en los dos idiomas.

3. **Lo común, resuelto una vez.**
   - El perfil sigue saliendo de `ProfileArg::resolve`, con sus reglas (`sharing_profile`, `preferences_profile`, el de cada acción), ahora con `?`. `ProjectsAction::profile` (nueva, en `cli.rs`, como `MemoriesAction::profile`) dice cuál toma cada acción de `projects`, y los 9 brazos pasan a uno.
   - `language`, hoy en `main.rs:107`, pasa a `crates/clawg-cli/src/preferences.rs` para que la usen los módulos. **Se lee en el mismo punto que hoy**: `language_in` escribe en stderr una línea por un `preferences.json` inválido, y leerlo antes de un paso que puede fallar sumaría esa línea donde hoy no sale. `export` lo sigue leyendo después de resolver el proyecto, y `secrets` sin `--mask-reasons` después de leer el store.
   - `Running` (nuevo, `crates/clawg-cli/src/running.rs`): el daemon que un comando encontró corriendo, sin arrancar ninguno, con el idioma y el perfil del comando para decir que no lo encontró (`Running::unavailable`, con `memories::explain_unavailable`). Reemplaza las líneas repetidas de los 13 brazos.

4. **Qué brazo se muda.** Un brazo pasa a una función de su módulo cuando hace algo propio además de resolver lo común, llamar una función y mostrar lo que devuelve: elige entre caminos, traduce un error, saca el código de salida del resultado o da un segundo paso. Se mudan `Status`, `Stop`, `Backup` (a un módulo nuevo, `backup.rs`), `Restore`, `Export`, `Import`, `Secrets`, `Purge`, `Metrics`, `Projects` y `Path` (a `user_path.rs`). Quedan en `dispatch`, porque con lo común resuelto ya son una llamada: `Serve`, `Hook`, `Mcp`, `Undelete`, `Outbox`, `Memories`, `Sharing`, `Preferences`, `Doctor` (su código sale de `Summary::fails`, que es de su módulo), `Update` y `Setup`.

5. **La función de un comando devuelve lo que se imprime, y `dispatch` lo imprime**, como ya hacen `update::run` y `setup::run` con `(String, u8)`. Así la salida se prueba sin capturar stdout, y el orden entre stdout y stderr no cambia: `import` sigue imprimiendo su informe y después la nota. Donde el módulo ya tiene un `run` con otra firma (que sus tests usan), la función del comando se llama `command`; para que se lean igual en `dispatch`, todas se llaman `command`, salvo `secrets::purge`, que es la de `clawg purge` en el módulo de `secrets`.

6. **Commits chicos**, en este orden: primero `status`, con su test en rojo; después `CliError`; después `stop` y `Running`; después un commit por comando. Cada uno se comprueba con los tests de los archivos que cambia y con `crates/clawg-cli/tests/binary.rs` cuando toca un comando que ese archivo corre.

## Alcance

1. `crates/clawg-cli/src/error.rs`: `CliError`, con un test por la regla de su texto.
2. `dispatch` y `main` con `Result<ExitCode, CliError>`.
3. `status::command`, `stop::command` con `stop::explain` y `stop::report`, `backup::command`, `restore::command`, `export::command`, `import::command`, `secrets::command` y `secrets::purge`, `metrics::command`, `projects::command` y `user_path::command`, cada uno con tests sin `main`.
4. `running.rs` con `Running`, `preferences::language` y `ProjectsAction::profile`.
5. `docs/reference/data-profiles.md`, que nombra `language` en `main.rs`; la fila 112 de `openspec/README.md`.
6. Lo que el CI del pull request marque en `windows-latest`: `user_path::command` y lo que queda de `main.rs` bajo `cfg(windows)` no se compilan en Linux.

## Fuera de alcance

- Cambiar un comando: sus argumentos, sus mensajes, su salida o su código de salida. Tampoco se traduce lo que hoy no se traduce (`name the file to import, or --undo N`, la negativa de `purge` sin memoria ni borrado, `clawg path`).
- La forma de `Command` y de sus acciones en `cli.rs`: la ayuda de `clap` no cambia.
- `serve::run` (`crates/clawg-cli/src/daemon/serve.rs:105`), que tiene otro problema: es el arranque del daemon.
- `clawg-core`, `clawg-eval` y cualquier otro crate.
- `clippy::pedantic` (`too_many_lines` entre ellos).
- Subir la versión y archivar.

## Cómo se mide el éxito

- `crates/clawg-cli/tests/binary.rs`, que corre el binario real, pasa **sin editarse**: `git diff a7f7337 -- crates/clawg-cli/tests/binary.rs` sale vacío. Si hubiera que tocarlo, algún comando cambió.
- **Test en rojo:** `status::command` se prueba sin `main` (no compila hoy): sin daemon sale con 1 y un informe con `"running": false` y el store `missing`; con un daemon en proceso sale con 0 y lleva su `health`.
- Cada comando que se muda tiene un test sin `main` de lo que el brazo decidía (detalle en la [spec](spec.md)).
- Un error que hoy sale con `e.to_string()` sale con exactamente ese texto: el `Display` de `CliError` es el de su fuente (test en `error.rs`), y una corrida del binario de antes y del de después sobre los mismos casos, en un home temporal, en `en` y `es`, da la misma salida, el mismo stderr y el mismo código.
- La suite pasa entera; `cargo clippy --workspace --all-targets -- -D warnings` sale con 0 en Linux, y en Windows por el CI del pull request.

## Datos y vuelta atrás

No toca `memory.db`, `metrics.db`, `preferences.json` ni ningún archivo del directorio de datos, ni lo que ve el modelo (`agent_contract.json` no cambia). Volver atrás es revertir los commits.
