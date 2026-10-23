# cli-language — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M–L (la parte B de la [exploración de 78](../language-preference/exploration.md#alcance-candidato)) | language-preference (78): `Lang` y `preferences.json` | ver [`state.yaml`](state.yaml) |

## Problema

Con 78 la persona elige su idioma en `preferences.json`, y el agente le habla y guarda en él. Lo que clawg mismo le muestra sigue en inglés:

1. **`clawg doctor`** escribe cada `message` en inglés (`crates/clawg-cli/src/doctor/checks.rs`, de `path` a `invariants`, más `doctor/mcp.rs`), aunque el chequeo `preferences` ya sepa que la persona eligió `es`.
2. **Los informes de los comandos** que cambian algo: `update` (`update/report.rs`, `success` y `failure`, y las líneas de `update/mod.rs`), `setup` (`setup.rs`, `report`), `secrets` y `purge` (`secrets.rs`), `export` e `import` (`export.rs`, `report`; `import.rs`, `report`), `restore`, `undelete`, `outbox`, `projects`, `sharing`, `metrics`, `memories`, `stop` y `path` (`main.rs`, `stop` y `run_path`).
3. **Las frases que enmarcan un fallo**: «no clawg daemon is running for profile dev; start it with …» (`memories.rs`, `explain_unavailable`), el aviso de versión (`version_warning`), el `--yes` que pide `purge` (`secrets.rs`, `confirm`) y los que ya tiene `clawg preferences` (`preferences.rs`), que fue el primer uso del molde.

La persona que eligió `es` lee entonces dos idiomas: el agente en español y `clawg doctor` o `clawg update` en inglés.

## Decisiones

Las decisiones 6 y 10 de la exploración ya están cerradas: se localiza lo que clawg compone y se deja en inglés lo que viene de afuera (6.a), y B es un cambio aparte (10.a). Esta propuesta las concreta:

1. **El idioma se lee una vez por comando**, en `main.rs`, del directorio de datos del perfil: `preferences::language_in(base_dir)`, y `Lang::En` sin preferencia. Un archivo inválido cuenta como sin preferencia y `language_in` lo registra una vez en stderr, como hoy en el puente y los hooks; `doctor` además lo nombra. Ningún comando falla por él.
2. **Un `match` sobre `Lang` por frase**, en la función que compone el informe, que recibe el `Lang`. Sin tabla, sin dependencia, sin macro: el compilador exige cada frase en los dos idiomas, y la frase en inglés queda escrita al lado de su par.
3. **Sin preferencia, la salida es la de hoy, byte a byte.** Las ramas `Lang::En` son los textos actuales, así que los tests actuales no cambian sus textos esperados: sólo pasan `Lang::En`.
4. **Lo que no cambia de idioma** (6.a):
   - los identificadores: `check_id`, `reason_code`, `severity`, `evidence`, `safe_next_step`, las claves de cada JSON (`status`, `doctor`, `metrics report`), los valores de `state`, los códigos de salida y los comandos entre comillas invertidas;
   - la causa técnica: el `Display` de los errores tipados (`thiserror`, en `clawg-core` y en `clawg-cli`), los errores de SQLite, del sistema, de HTTP y del servidor de releases, que se muestran tal cual dentro de la frase que los enmarca;
   - la ayuda y los errores de `clap`, los logs, el arranque del daemon (`clawg serve` sólo devuelve errores tipados, y el que arranca un puente o `update` escribe en `serve.log`, que se lee junto con los logs) y `scripts/install.ps1`;
   - lo que un comando lista como datos: los ids, los tipos de memoria, el texto de una fila, las etiquetas de `memories list|show` que son nombres de campo.
5. **Plurales.** Las frases con número (`secrets`, `export`, `import`, `projects`) eligen la forma por número en cada idioma; donde el español pide concordancia que el inglés no (`1 memoria guarda`, `2 memorias guardan`), la rama `Lang::Es` la escribe aparte.
6. **El español es neutro**, con imperativo de tú en los pasos («ejecuta `clawg stop`»), como el resto de lo que lee la persona (`preferences.rs`).

## Alcance

1. `main.rs`: el idioma de cada comando y su paso a los informes.
2. `doctor`: los `message` de `checks.rs` y `mcp.rs`, con `Lang` como argumento de cada chequeo.
3. `update`: `report.rs` (`success`, `failure`, `install_dir_access`), las líneas de `update/mod.rs` y `--check`. Las causas de `release.rs`, `signature.rs`, `effects.rs` y `client.rs` quedan en inglés.
4. `setup`: `report`.
5. `secrets` y `purge`: `report`, `mask_report`, `purge_report`, `confirm`, `confirm_mask_reasons` y `explain`.
6. `export` e `import`: sus informes.
7. `restore`, `undelete`, `outbox`, `projects` (`add` y `move`), `sharing`, `metrics` (los errores que compone), `memories` (los avisos y `explain_unavailable`), `stop` y `path`.
8. Los tests: cada informe en los dos idiomas, y un test que recorre los chequeos de `doctor` con los mismos hechos en `en` y `es` y compara todo menos `message`.
9. Las specs `doctor`, `setup`, `self-update`, `secrets`, `portability`, `preferences` y `.claude/CLAUDE.md`.

## Fuera de alcance

- Lo que lee el agente (protocolo, herramientas, respuestas del puente, contexto, hooks): es agent-language (80).
- Traducir errores tipados (decisión 6.b) y la ayuda de `clap` (6.c).
- Más idiomas que `es` y `en`.
- El JSON de `clawg status` y de `clawg metrics report`: no tiene prosa más allá de `store.message`, que es el texto de un error tipado.

## Datos guardados

Ninguno. El cambio no toca `memory.db`, `metrics.db`, `preferences.json` ni ningún archivo del directorio de datos; sólo lo que se imprime.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- con `es`, cada informe de la lista de alcance sale en español, y con `en` o sin preferencia sale igual que hoy (los tests actuales, con `Lang::En`, no cambian sus textos);
- los hallazgos de `doctor` de los mismos hechos tienen el mismo `check_id`, `severity`, `reason_code`, `evidence`, `safe_next_step` y `requires_confirmation` en los dos idiomas, y `message` distinto;
- `clawg doctor` y `clawg stop` del binario real con `es` en un home temporal hablan en español, y con un `preferences.json` inválido hablan en inglés y salen con el código de siempre;
- las frases con número concuerdan en los dos idiomas (uno y varios).
