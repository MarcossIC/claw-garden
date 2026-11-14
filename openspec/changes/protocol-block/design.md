# protocol-block — Diseño

## Dónde queda cada pieza

| Pieza | Archivo | Pura |
|---|---|---|
| Las variantes y el texto corto (`Variant`, `protocol::short`) | `crates/clawg-cli/src/mcp/protocol.rs` | sí |
| El sello: escribirlo, leerlo (`Stamp`, `stamp_line`, `read_stamp`) | `crates/clawg-cli/src/setup.rs` | sí |
| `block(eol, language, variant, stamp)` y la frescura (`Freshness`, `freshness`) | `setup.rs` | sí |
| La variante que planea `install_block` | `setup.rs` | sí |
| `--protocol` | `crates/clawg-cli/src/cli.rs`, `crates/clawg-cli/src/main.rs` | parseo de clap |
| El hallazgo (`checks::protocol_block`) | `crates/clawg-cli/src/doctor/checks.rs` | sí |
| Leer el bloque para `doctor` | `crates/clawg-cli/src/doctor/mod.rs` | I/O |
| La línea del aviso (`block_notice`) y dónde va en `session_start_text` | `crates/clawg-cli/src/hook.rs` | sí |
| Leer el bloque en el hook, dentro del tope | `hook.rs` (`respond_reading`) | I/O |
| El brazo `a3-short` | `crates/clawg-eval/src/bin/agent-eval/arm.rs` | sí |

`doctor` y el hook leen el archivo con lo que ya existe (`setup::locations`, `setup::find_block`, `setup.rs:612`, `:670`) y deciden con `freshness`. La versión propia entra como argumento (`clawg_core::build_info::version()`, parseada con `crate::update::version::Version::parse`), así los tests eligen la versión sin tocar el build.

## D1. Detectar y avisar, no reescribir

**Decisión:** `doctor` y `session-start` dicen que el bloque difiere y nombran la orden; sólo `setup` escribe.

| Alternativa | Por qué no |
|---|---|
| `clawg serve` lo refresca al arrancar | Escribe un archivo de la persona sin una orden y sin que nadie vea el reporte. El bloque es uno para los dos perfiles (`setup.rs:670-683`, `:234-245`): un daemon `dev` de una rama pisaría el de `prod`, y el de `prod` lo volvería a pisar |
| El hook lo refresca | Corre con 3 s de tope y en paralelo en varias sesiones, cuando el cliente ya leyó el archivo; la escritura de una sesión no le sirve a esa sesión |
| `clawg update` lo refresca | El binario que actualiza es el viejo y no conoce el texto nuevo; tendría que lanzar el `setup` del nuevo, que también toca servidor y hooks y falla sin `claude` en el PATH |

Lo que el aviso gana sobre las tres: llega a la sesión que está leyendo el bloque viejo y le dice que siga las instrucciones del servidor, que son las de este binario. Eso arregla el gasto de `resolve_conflict` desde la primera sesión, corra la persona `setup` o no.

## D2. El sello: versión y variante

**Decisión:** la línea siguiente a `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` es `<!-- clawg 0.88.0 -->`, o `<!-- clawg 0.88.0 protocol=short -->` cuando la persona eligió la variante. La frescura compara el resto del bloque, sin esa línea.

| Alternativa | Tradeoff |
|---|---|
| Sin sello, comparar el texto | Lo más simple, pero no dice en qué dirección está la diferencia. Con un `dev` y un `prod` instalados, los dos verían el bloque «viejo» y cada orden pisaría al otro. Tampoco hay dónde recordar la variante elegida |
| Un hash del texto | Dice que difiere, que ya se sabe comparando, y no ordena versiones |
| La versión en el marcador de apertura | Un binario anterior busca el marcador exacto (`find_block`, `setup.rs:615`): no lo encontraría, y `setup` agregaría un segundo bloque |
| **Una línea propia después del marcador** | Los binarios anteriores lo encuentran y lo reescriben enteros; cuesta una línea (unos 10 tokens) |

Como el sello no cuenta al comparar, una versión nueva con el mismo texto no reescribe nada: `setup` da `Unchanged` y el sello conserva la versión que escribió ese texto. `newer` sólo sale cuando el texto difiere y el sello es posterior a este binario; ahí la orden de `setup` bajaría el protocolo, así que `doctor` no la ofrece y el hook calla.

## D3. El aviso, en cada inicio de sesión

**Decisión:** `session-start` de Claude Code imprime la línea en cada inicio, con cualquier `source`, cuando el bloque está `stale`, y nunca en `user-prompt-submit`. No guarda si ya avisó.

| Alternativa | Por qué no |
|---|---|
| Una sola vez, guardando en el directorio de datos que ya avisó | Cada sesión nueva vuelve a leer el bloque viejo, y es cada sesión la que necesita la corrección «sigue las instrucciones del servidor». El hook no tiene estado entre eventos, y sumarle uno para esto no compensa |
| En las instrucciones del servidor | Lo fija el contrato del agente, se arma al `initialize` y lo lee también Cursor, que no tiene bloque |
| Sólo en `doctor` | Es lo de hoy en el caso con preferencia, y nadie corre `doctor` después de un `update` |

La línea es, en inglés:

```
clawg: the memory protocol block in the user CLAUDE.md is not the one this clawg (0.88.0) writes, and may contradict this server's instructions; where they differ, follow the server's instructions. Tell the user once that `clawg setup claude-code --profile prod` rewrites it.
```

y en español:

```
clawg: el bloque del protocolo de memoria del CLAUDE.md del usuario no es el que escribe este clawg (0.88.0), y puede contradecir las instrucciones de este servidor; donde difieran, sigue las del servidor. Dile una vez a la persona que `clawg setup claude-code --profile prod` lo reescribe.
```

Va después de la línea del idioma y antes del contexto: como la del idioma, es la voz de clawg y no evidencia. No nombra la ruta, que alarga la línea y que `setup` ya conoce. La lectura del archivo va en un hilo propio con el mismo tope que la preferencia (`LANGUAGE_WAIT`, `hook.rs:57`, leído con `read_by`, `:416`), en paralelo con ella; un archivo que no se leyó a tiempo cuenta como sin bloque.

## D4. Tres variantes de un solo archivo

**Decisión:** `Variant::{Full, Short, Off}`. `Full` es `protocol::text(language)`, como hoy. `Short` sale del mismo texto: el título, una línea fija por idioma («The rules for clawg's tools come with its server's instructions; this block keeps only what they leave out.» / «Las reglas de las herramientas de clawg vienen con las instrucciones de su servidor; este bloque guarda sólo lo que les falta.») y las secciones «Session summary» y «After a compaction» («Resumen de sesión», «Después de una compactación») cortadas por su encabezado. `Off` no tiene texto: sólo los marcadores y el sello.

| Alternativa | Tradeoff |
|---|---|
| Dos archivos más, `protocol.short.md` y `protocol.short.es.md` | Fáciles de leer, pero repiten la sección del resumen y se desalinean con el tiempo |
| **Cortar las secciones del archivo entero** | Una sola fuente; un test fija que las dos secciones existen en los dos idiomas |
| `Off` sin marcadores | El archivo quedaría como sin bloque y la siguiente corrida de `setup` escribiría el de por defecto, pisando la elección |

Por qué esas dos secciones: es lo único del protocolo que no dicen ni la apertura ni las descripciones fijadas en el contrato, y el hook de una compactación remite a ellas («the memory protocol asks for a `session_summary`», `hook.rs:141-144`). Medido sobre los archivos de hoy, el bloque corto ronda los 1.020 bytes en inglés y los 1.160 en español, contra 10.691 y 11.442 del bloque entero (unos 300 tokens contra unos 3.550).

## D5. La variante elegida vive en el sello

**Decisión:** `--protocol V` escribe la variante `V` con `protocol=V` en el sello. Sin la opción, `setup` lee el sello del bloque instalado: si trae `protocol=`, mantiene esa variante; si no, escribe la de por defecto, sin `protocol=`. `--protocol` se rechaza con `--remove`, con `--local`/`--path` y con `setup cursor`.

| Alternativa | Tradeoff |
|---|---|
| Un campo en `preferences.json` | Es un archivo por perfil y el bloque es uno para los dos; cambia un archivo versionado ([`specs/preferences`](../../specs/preferences/spec.md)) |
| Guardar siempre la variante en el sello | Cuando cambie el valor por defecto, quien nunca eligió quedaría atado a la variante de antes |
| **Sólo la elección explícita** | Quien eligió la conserva; quien no eligió sigue el valor por defecto, también cuando cambie |

No hay forma de volver a «sin elección» salvo borrar la línea del sello. Sumar un valor más a `--protocol` para eso no tiene un uso todavía.

## D6. La regla de decisión del valor por defecto

Se fija ahora, antes de correr, y no se cambia después de ver resultados.

**Brazos:** `a3` (bloque entero), `a3-short` (bloque corto, PB7) y `l0` (sin bloque, `crates/clawg-eval/src/bin/agent-eval/arm.rs:32-33`), los tres con el mismo `setup`, las mismas memorias y el ruido de 15 reglas, con el mismo modelo, esfuerzo y cliente.

**Tareas y corridas:** las tres tareas largas de [agent-eval-sessions](../agent-eval-sessions/proposal.md#las-tareas-nuevas) (129) donde el bloque puede pesar: `requests-timeout-decision-relay` (R, en las formas que 129 corre bajo `a3`), `requests-redact-decision-revised` (D partida) y `requests-history-owner-convention` (la K larga), 3 corridas por celda, como 129 las corre bajo `a3`. Las corridas de `a3` son las de 129 si se corren con el mismo build; si no, se corren otra vez con el de `a3-short` y `l0`. `shelf-eza-missing` queda afuera: mide un error de shell, no el protocolo. Las celdas de `a3-short` y `l0` son 18 corridas, con los topes de `--max-budget-usd` de 129 y un tope propio de 15 USD. Valen las reglas de lectura de 129 tal como están: sin «casi», corridas fijas, las mismas causas de exclusión y la fase que se detiene con más de 2 exclusiones ([regla de decisión de 129](../agent-eval-sessions/proposal.md#la-regla-de-decisión)). Si 129 cambia los nombres de las tareas o los brazos, la regla vale para las que equivalgan.

**Métricas**, por brazo, sumadas sobre sus corridas:

| | Qué cuenta |
|---|---|
| E | corridas que aprueban los checks de su tarea |
| B | sesiones que llaman a `search_memory` o `recall_context` antes de su primer `Write` o `Edit`, entre las que necesitan un dato de antes: la segunda de R, la tercera de D y la K larga |
| G | hechos nacidos en una sesión que alguna sesión anterior a la que los usa guardó en clawg (la clase contraria a `no guardado` de H3 de 129) |
| S | sesiones con una siguiente en su tarea (la primera de R, las dos primeras de D) que guardan un `session_summary` con todas las secciones de «Session summary» |
| J | `resolve_conflict` por cada save con cercanas |
| T | mediana, por sesión, de los tokens de entrada de todos sus pedidos, caché incluida |

**Una variante V pasa** frente a `a3` si cumple las seis:

1. E(V) ≥ E(`a3`) − 1, y ninguna tarea pasa de aprobar en todas sus corridas a no aprobar en ninguna;
2. B(V) ≥ B(`a3`) − 1;
3. G(V) ≥ G(`a3`) − 1;
4. S(V) ≥ S(`a3`) − 1;
5. J(V) ≤ J(`a3`) + 0,5;
6. T(V) < T(`a3`).

**Qué se decide:** el valor por defecto pasa a ser la variante más chica que pase, en el orden `off`, `short`; si ninguna pasa, queda `full`. No se agregan corridas para desempatar.

**Por qué así:** con 9 corridas por brazo la campaña no puede probar que dos brazos son iguales; sí puede descartar una pérdida de más de una. El ahorro no necesita prueba: el bloque son unos 3.550 tokens en cada pedido, y la métrica 6 sólo pide que aparezca en la sesión entera. S va aparte porque es lo único que sólo dice el bloque, y es la que puede separar `short` de `off`. J está porque el bloque viejo pedía los veredictos `compatible`: una variante que vuelva a pedirlos no pasa.

**Lo que no decide:** si `off` gana, la forma del `session_summary` deja de llegar al agente por cualquier canal; el cambio que aplique el resultado decide si eso pide moverla a otra parte (la apertura no tiene lugar: 2.030 de 2.048 caracteres, `server.rs:2948-2956`; llevarla a los 4.096 que, según su documentación, corta el cliente actual dejaría afuera a los anteriores a 2.1.296).

## Lo que cambia en el contrato del agente

Nada. Las instrucciones siguen siendo la apertura (`server.rs:2069`) y las descripciones no cambian, así que `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`server.rs:3421`, `:3444`) tienen que pasar sin `CLAWG_BLESS=1`. El aviso va por el hook, que el contrato no cubre, y el bloque corto es un texto de `setup`. Si la tarea de verificación necesita regenerarlos, es que algo cambió de más.
