# context-budget — Diseño

## D1. El presupuesto se cumple recortando desde el final, sobre el prompt entero

`build_memory_prompt` arma las partes como hoy (workspace, hits partidos en RECENT y LONG-TERM, conflictos ya ordenados y con su tope de cinco) y se las da a una función pura que decide qué entra. Esa función renderiza el prompt completo, con el bloque `BUDGET` incluido, y mientras supere el presupuesto quita la última pieza en orden de prioridad:

1. el último hit de LONG-TERM, después el de RECENT;
2. el primer hit que queda solo en su bloque no se quita: se recorta su texto hasta que entre, y se quita sólo si no le queda texto;
3. después, los pares de conflictos desde el último, que pasan a contar en `(+N conflictos más sin mostrar)`;
4. el workspace y el bloque `BUDGET` no se quitan nunca.

Como cada paso renderiza todo, el pie, la nota `SHARED` (que sale sólo si queda un hit ajeno) y el texto que un conflicto suma cuando su hit se fue entran en la cuenta sin estimaciones. Son a lo sumo diez hits y cinco pares: quince renders de unos pocos KB.

Alternativas:

| | A favor | En contra |
|---|---|---|
| **Recortar desde el final sobre el render completo (elegida)** | Exacta; pura; sin reservas estimadas | Varios renders por llamada (despreciable) |
| Reservar el pie y repartir por bloque | Un solo render | El pie depende de lo cortado y la nota de conflictos de los hits: habría que estimar, y una estimación corta pasa el tope |
| Porcentajes fijos por bloque | Simple | Un bloque vacío desperdicia su parte (exploración, «Descartado») |

## D2. El workspace es atómico

El workspace no se recorta. Las preferencias y la sección USER no se tocan por diseño (`workspace.rs:392-394`), y las decisiones y skills que el AUTO imprimió ya están en `EmittedIds`: la búsqueda las saltea (`prompt_builder.rs:178`). Si el presupuesto las quitara del prompt, no llegarían por ningún canal, que es lo que `EmittedIds` existe para impedir (`workspace.rs:192-201`). El AUTO ya tiene su tope de 6.000 bytes salvo las preferencias.

Cuando el workspace y el pie solos superan el presupuesto, se entregan igual y el pie dice cuántos bytes ocupa el contexto completo: el agente sabe que hubo un recorte y con qué `budget` no lo habría.

## D3. Conflictos: se buscan antes y se imprimen sin citar lo que no está

Los conflictos se piden, como hoy, sobre los hits candidatos y las filas del workspace (`prompt_builder.rs:211-234`), en una sola consulta antes de recortar. Al imprimir, un lado se cita por `#id` sólo si su hit quedó en el prompt; si el hit se cortó, el lado lleva su texto, igual que hoy un lado que nunca fue hit.

| | A favor | En contra |
|---|---|---|
| **Antes del recorte, posiciones después (elegida)** | Una consulta; el recorte es puro; un conflicto no se pierde por el presupuesto de los hits | Puede mostrar un conflicto sobre una fila que el presupuesto sacó de los hits (con su texto) |
| Después del recorte | Sólo conflictos de lo visible | Una consulta por paso de recorte, o perder conflictos; el recorte deja de ser puro |
| Antes, citando por id | Más corto | Cita un id que no se imprimió (exploración, «Orden de corte») |

`rank_conflicts` no cambia: con posiciones de los hits que quedaron, un par cuyo hit se fue ordena como uno de workspace.

## D4. Lo que se cortó, por id

El bloque `=== BUDGET ===` sale sólo si algo se cortó o el workspace no entra:

```
=== BUDGET ===
Cut to fit 8000 bytes; the full context is 11234 bytes. Left out: #40, #41, procedural/3; shortened: #12. read_memory returns any of them whole.
```

Un hit recortado termina en ` [truncated]`. El separador propio hace que el parser de `rag-eval --agent-view` no lo lea como texto de un hit (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs:83-94`). Los ids de otra tabla llevan su kind, como en la línea del hit; los conflictos cortados se cuentan en su propio bloque.

El corte de un texto cae en un límite de carácter UTF-8: retrocede desde el largo pedido hasta `is_char_boundary`.

## D5. El estado de la búsqueda viaja aparte del prompt

`build_memory_prompt` devuelve el texto y un `Retrieval`: buscó, no había embedder, `embed` falló (con su `EmbedError`) o la búsqueda falló (con su texto). El daemon lo traduce a `retrieval` en la respuesta y lee el estado de `embedder_status`, la misma fuente que `ready_embedder` (`api.rs:868-875`); un `EmbedError::NotReady` (la recarga de un modelo ocioso que falló) se lee como `embedder_not_ready`, igual que `embed_failure` (`api.rs:860-865`):

```json
"retrieval": { "state": "searched" }
"retrieval": { "state": "embedder_not_ready" }
"retrieval": { "state": "embedder_failed", "detail": "...", "retryable": true }
"retrieval": { "state": "search_failed", "detail": "..." }
```

El puente antepone una frase cuando no es `searched`; con prompt vacío la frase sola reemplaza `no context yet`. El aviso queda fuera del prompt, como el recordatorio de `synthesize` (`specs/synthesis` R8), para que `rag-eval --agent-view` no lo cuente. El presupuesto cubre el prompt; el puente le suma a lo sumo dos frases fijas.

No se responde `503`: el workspace llega y es útil.
