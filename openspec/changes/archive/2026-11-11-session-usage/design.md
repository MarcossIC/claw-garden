# session-usage — Diseño

Las seis decisiones de fondo están en la [exploración](exploration.md#decisiones-2026-11-10) y no se reabren. Acá van las de implementación, con las alternativas que se descartaron. Las citas son `ruta:línea` de `develop` en `68b8851`.

## D1. Dónde vive el registro en memoria

El registro (`SessionLog`, `crates/clawg-core/src/metrics/sessions.rs`) cuelga del `Recorder` del runtime (`Recorder::sessions`).

- El `Recorder` ya llega a los dos lados que lo necesitan sin tocar ninguna firma: los handlers lo tienen en `app.runtime.metrics` y el escritor lo recibe en `Writer::start` (`crates/clawg-cli/src/daemon/metrics.rs:146`). Ya guarda una cola que espera al escritor, la de los eventos (`take_events`).
- `Recorder::set_writing` prende y apaga el registro: con la escritura apagada no acumula nada en memoria.

**Descartado:** un campo en `AppCtx`. Se construye en trece lugares, la mayoría tests, y el escritor nace antes que `AppCtx` (`crates/clawg-cli/src/daemon/serve.rs:220-238`). **Descartado:** dentro de `SessionRegistry` (`daemon/sessions.rs`), que es de `clawg-cli`: las reglas quedarían lejos de la tabla que las guarda.

`SessionRegistry` no cambia. Sigue siendo lo que sella los saves, con su regla de hoy; el registro nuevo lo consulta para atribuir, y nada de lo que se guarda en `memory.db` depende del registro nuevo.

## D2. Las reglas son puras

`SessionLog` no lee el reloj ni toca disco: cada método recibe la hora (`at_ms`) y el escritor le pide lo que cambió (`take`). La regla de las 6 horas es una función de dos horas (`idle_closed`). Los tests de SU2, SU3 y SU7 corren sin base ni reloj, con las horas como datos.

Una fila se identifica por (`session_id`, `started_at_ms`). No hace falta pedirle un id a SQLite antes de escribir, y el estado entero de una fila se guarda con `INSERT OR REPLACE`: la copia en memoria de una fila abierta es la verdad, y el archivo la alcanza con cada minuto. Una fila que se abre en el mismo milisegundo en que cerró la anterior del mismo id empieza un milisegundo después, para no pisarla.

Los contadores con nombre (arranques por `source`, llamadas por herramienta, saves por tipo) van a una tabla aparte, `session_count`, y no a una columna por nombre ni a un JSON. Sumar una herramienta al puente no cambia el esquema, y se pueden consultar con SQL.

**La regla de las 6 horas, en tres lugares, con la misma función:**

| Dónde | Cuándo |
|---|---|
| `SessionLog`, antes de contar un evento o una actividad | es lo que da la hora exacta del corte, entre dos escrituras |
| el escritor, con cada minuto y al arrancar | deja el archivo al día y saca de la memoria las filas que nadie retomó |
| el reporte, al leer | una fila que el archivo todavía tiene abierta (el daemon no corre, o se mató) se lee como `idle` |

## D3. El nombre de la herramienta: un aviso del puente al terminar

El puente manda `POST /v1/sessions/calls` cuando la herramienta terminó, con el agente, el proyecto de la ventana, el nombre y si falló.

| Alternativa | Por qué no |
|---|---|
| Una cabecera con el nombre en cada petición de la herramienta, y un middleware del daemon que cuenta | Una herramienta hace varias peticiones (`save_memory` resuelve el proyecto y después guarda; `memory_status` hace más de una), así que habría que desduplicar con un id de llamada y guardar los ids recientes por sesión. El daemon vería el estado de cada petición, no el resultado que recibió el modelo: una herramienta que el puente rechaza antes de pedir nada no contaría, y una petición auxiliar rechazada contaría como fallo |
| Contar en el handler de la «ruta principal» de cada herramienta | Pide una tabla herramienta→ruta que se desactualiza en silencio, y cinco herramientas van por id de memoria (`read_memory`, `update_memory`, `delete_memory`, `restore_memory`, `resolve_conflict`), sin proyecto en la petición; `search_project` y `answer_suggestion` ni siquiera mandan agente |
| `PostToolUse` | Descartado en la exploración: un proceso más por llamada |

El aviso cuesta una petición por loopback. Se espera, acotada a `CALL_REPORT_WAIT` (500 ms), antes de devolver el resultado: así el orden de lo contado es el de las llamadas y un test lo lee sin esperar. No usa `ClientHandle::get`, que puede esperar a un daemon que arranca: mira el cliente que ya hay (`ClientHandle::peek`) y, si no hay, no avisa.

**El proyecto del aviso es el de la ventana,** no el que nombra la llamada: el de la única carpeta abierta (`roots` del cliente, o el directorio del puente). Es la clave bajo la que el hook registró la sesión con su `cwd` (`project_of`, `crates/clawg-cli/src/hook.rs:415`), así que una llamada que guarda en otro proyecto por nombre sigue contando para la sesión que la hizo. El id sale de lo que el puente ya sabe (`known_ids`: el que el daemon le dio por esa carpeta, o el derivado de su raíz), sin ninguna petición, y el daemon lo sigue hasta su proyecto vivo.

**Descartado:** registrar la carpeta para pedirle el id al daemon, como hace el hook. Registrar un proyecto escribe en `memory.db`, y una llamada que hoy no registra ninguno (`delete_memory`, un `search_memory` con `all_projects`) pasaría a registrar uno sólo para ser contada. El costo es el límite de la spec: una carpeta cuyo proyecto no es el derivado de su raíz no se atribuye hasta que una herramienta la registra.

Para saber qué carpeta hay abierta, las cinco herramientas que no les pedían las carpetas al cliente ahora se las piden (`asking`): un `roots/list` más en cada una, sólo a los clientes que declaran carpetas.

**El conjunto cerrado** es `TOOLS` (`crates/clawg-cli/src/daemon/sessions.rs`), los trece nombres. Un test del puente comprueba que es exactamente la lista de su router, así que sumar una herramienta sin sumarla ahí falla. El daemon pasa lo que recibe por `tool_label`, que devuelve el nombre conocido o `other`: lo que se guarda nunca es el texto que llegó.

## D4. Los saves y las búsquedas vacías se cuentan en el daemon

`POST /v1/memories` ya lee la sesión vigente del agente y del proyecto para sellar el save (`crates/clawg-cli/src/daemon/api.rs:1445`). Con ese mismo id cuenta el save cuando el outbox lo acepta (`Accepted::Queued`; no cuando la respuesta es la de un `operation_id` cuyo save sigue esperando o falló), con el tipo que la petición declara. No espera al worker: un save que contesta `202` también es un save aceptado, y su tipo declarado es lo único que el handler sabe siempre. Un save sin tipo cuenta como `unspecified`.

`POST /v1/search` cuenta una búsqueda vacía cuando, después del corte por puntaje, no devuelve ningún hit. La atribuye con el proyecto de la petición, como el save.

**Descartado:** que el puente lo diga en su aviso. El puente no ve el tipo que quedó ni cuántos hits cortó el daemon sin volver a leer su propia respuesta.

## D5. El cuerpo de los hooks

`POST /v1/sessions` suma tres campos opcionales (`event`, `source`, `client`) en lugar de una ruta por evento. Un hook de un clawg anterior sigue registrando la sesión, sin contar; un hook nuevo contra un daemon anterior manda campos que ese daemon ignora (`SessionRequest` no rechaza campos desconocidos, `api.rs:407`).

El cierre va a una ruta propia, `POST /v1/sessions/end`, porque no lleva proyecto: `session-end` no puede llamar a `POST /v1/projects`, y el id de la sesión alcanza para encontrar la fila. Contra un daemon anterior da 404 y el hook no hace nada.

`session-end` no pasa por `answer` (`hook.rs:429`): no lee la carpeta, ni el idioma, ni el contexto. Lee la entrada, busca el daemon y manda una petición, dentro de `SESSION_END_DEADLINE` (1 s), por debajo del presupuesto de 1,5 s que Claude Code le da a `SessionEnd`. No deja línea en `hook-runs.jsonl`, que es de `session-start`.

`ended_at_ms` es la hora en que el daemon recibe el aviso, no una que mande el hook: un solo reloj para toda la fila.

## D6. Unir y borrar proyectos

Los handlers le dicen al registro qué pasó (`SessionLog::merge_project`, `delete_project`) cuando el informe trae un `merge_id` o un `delete_id` y no es un deshacer. El registro lo aplica a sus filas en memoria y lo encola para el archivo; el escritor lo aplica antes de escribir las filas que cambiaron.

Para que un proyecto borrado no quede en el archivo hasta el cambio de minuto, el registro despierta al escritor (`SessionLog::changed`, un `Notify`), que escribe las sesiones en ese momento. El minuto de las series no se toca en esa escritura.

**Descartado:** una segunda conexión a `metrics.db` desde el handler. Serían dos escritores del mismo archivo, y el handler haría E/S bloqueante.

## D7. El reporte

La agregación es pura (`session_report::report`, sobre filas ya leídas), como la de `metrics report` (`crates/clawg-core/src/metrics/report.rs:82`). `clawg metrics sessions` abre el archivo en sólo lectura y la llama.

- **Agrupa por id de proyecto.** La exploración decía que, para nombrar proyectos, el reporte leería también `memory.db`. No lo hace: el daemon tiene el lock exclusivo del store, y leerlo sin daemon es lo que hace `export`, con sus propias reglas. El id alcanza para cruzar con `clawg projects list`.
- **El día** es la fecha local del comienzo de la fila, con el desfase que la fila guardó al abrirse (`utc_offset_s`), como las series usan el de su ejecución.
- **Una fila abierta** suma su duración hasta la última actividad y cuenta en `open`. El tiempo desde el último save sólo mira filas cerradas: de una abierta no hay cierre con el que medirlo.

## D8. Lo que no cambia

- `memory.db`, sus migraciones y `memory.source_session_id`.
- El contrato del agente: las herramientas, sus descripciones y sus esquemas. `TOOLS` es una lista de nombres del lado del daemon.
- `GET /v1/metrics`: no lista sesiones. El reporte lee el archivo.
- Las series: ninguna gana una etiqueta.
