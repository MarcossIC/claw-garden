# contract-diet — Diseño (el bloque `lean`)

Las decisiones llevan el número de la [exploración](exploration.md#decisiones-abiertas) cuando cierran una de sus decisiones abiertas (D2, D3, D5 y D6); las que nacen al diseñar siguen desde D9. D1 quedó cerrada con la clase (a). D4 queda como la recomienda la exploración: `lean` conserva enteras las secciones del resumen y nada se saca ni se mueve antes de memory-links (134). D7 y D8 no son de este cambio.

## Dónde queda cada pieza

| Pieza | Dónde | Pura |
|---|---|---|
| `Variant::Lean` y su nombre | `crates/clawg-cli/src/mcp/protocol.rs` (`Variant`, `ALL`, `as_str`) | sí |
| El texto de `lean` | `protocol::lean`, que junta la cabeza de `PROTOCOL` o `PROTOCOL_ES` con `protocol.lean.md` o `protocol.lean.es.md`, compilados con `include_str!` | sí |
| El bloque `lean` | `assemble` (`crates/clawg-cli/src/setup.rs:689`) | sí |
| La frescura y el sello | `freshness` y `Stamp::parse` (`setup.rs:735`, `:662`), sin cambios: ya comparan contra `assemble` de la variante del sello | sí |
| `--protocol lean` | `parse_protocol` (`crates/clawg-cli/src/cli.rs:26`), que ya lee `Variant::parse` | sí |
| El brazo | `Arm::A3Lean` (`crates/clawg-eval/src/bin/agent-eval/arm.rs`) | sí |

`doctor` (`crates/clawg-cli/src/doctor/checks.rs`) y el aviso de `session-start` (`block_notice`, `crates/clawg-cli/src/hook.rs`) usan `freshness` y no enumeran variantes: no cambian.

## D2. `lean` es una variante nueva

**Decisión:** `Variant::Lean`, la cuarta, junto a `full`, `short` y `off`. `full` no cambia ni una palabra.

| Alternativa | Tradeoff |
|---|---|
| Reemplazar el texto de `full` | El sello compara por texto: todo bloque `full` instalado pasaría a `stale` y cada sesión de cada máquina recibiría el aviso del bloque (unos 330 caracteres, `block_notice`) hasta correr `setup`. Y se elegiría antes de medir |
| **Variante nueva** | Quien tiene `full` sigue `current`; `lean` se elige con `--protocol lean` y se mide contra `full` con el mismo binario. Si la campaña la elige como valor por defecto, el aviso se paga una vez por máquina, que es para lo que está el sello |

## D3. La apertura no se toca

**Decisión:** la cabeza de `lean` es la de `full`, byte por byte, en cada idioma y con cada preferencia, y `opening` no cambia. Un test lo fija comparando las cabezas.

| Alternativa | Tradeoff |
|---|---|
| Sacar la apertura del bloque (es lo que hace `short`) | Ahorra unos 2.000 caracteres más, pero la campaña de protocol-block no puede separar qué perdió `short`: la apertura y ocho secciones salieron juntas, y `l0`, que tiene la apertura sólo en las instrucciones, perdió E, B y G. Descartado por la exploración (B6) |
| Achicar la apertura (B4) | Cambia lo que leen los tres clientes y el contrato fijado; sólo tiene sentido después de que `lean` pase, como un segundo brazo medido también contra `l0` |
| **Apertura intacta** | Si `lean` pierde una conducta, la causa está en las secciones comprimidas, no en la apertura |

## D5. El umbral de ahorro

**Decisión:** `lean` tiene que bajar T al menos un 10 %: T(`lean`) ≤ 0,9 × T(`a3`), además de las seis condiciones de D6 de protocol-block. Se fija antes de correr.

| Alternativa | Tradeoff |
|---|---|
| Sólo T(V) < T(`a3`), como D6 | Cualquier ahorro, por chico que sea, pagaría el riesgo de cambiar lo que el agente lee |
| 5 % | Está dentro del ruido de una campaña de 12 corridas por brazo |
| 15 % | Es lo que estimaba la exploración para un bloque de unos 6.000 caracteres; con las reglas enteras `lean` queda en unos 7.000 (D10) y se espera cerca del 10,7 %, así que sería decidir por adelantado que no pasa |
| **10 %** | Debajo, el ahorro no justifica ni el riesgo ni la campaña |

## D6. `a3-lean` es un brazo del arnés

**Decisión:** `Arm::A3Lean`, `"a3-lean"`, junto a `A3Short` en `ARMS` y en los predicados `has_clawg`, `has_setup` y `has_protocol`. El plan de `setup` lleva `protocol: Some(Variant::Lean)`; todo lo demás es `a3`.

| Alternativa | Tradeoff |
|---|---|
| Correr `a3` con `--protocol lean` a mano, fuera del arnés | La corrida no queda descrita por el plan, no tiene test que diga que difiere de `a3` sólo en el bloque, y las reglas de D6 se aplicarían a algo que el arnés no ve |
| **Brazo** | El plan lo arma `setup::plan`, como el de `a3-short`; un test fija que sólo cambia el `CLAUDE.md`, y la campaña corre con las mismas reglas de exclusión |

El predicado de `a3-short` (`protocol: (arm == Arm::A3Short).then_some(Variant::Short)`, `arm.rs:316`) pasa a un `match` de los dos brazos.

## D9. El texto de `lean`: la cabeza de `full` y un archivo de secciones

**Decisión:** dos archivos nuevos, `protocol.lean.md` y `protocol.lean.es.md`, con las diez secciones comprimidas y sin título ni apertura. `protocol::lean` toma la cabeza de `PROTOCOL` o `PROTOCOL_ES` y le suma el archivo de su idioma; con la preferencia `en`, la regla del idioma se reescribe con la misma función que usa `text`, que pasa a recibir el texto en lugar de leer `PROTOCOL` directamente.

| Alternativa | Tradeoff |
|---|---|
| Dos archivos completos, con la apertura copiada | Fáciles de leer, pero la apertura quedaría en dos lugares por idioma y un cambio de `protocol.md` la desalinearía; un test lo vería tarde |
| Secciones marcadas dentro de `protocol.md` (como las que corta `short`) | `full` tendría que llevar el texto de `lean`, y `full` no cambia (D2) |
| **Cabeza de `full` más un archivo de secciones** | La apertura tiene una sola fuente; las secciones de `lean` se leen como un texto propio. Lo que queda entero en `lean` (la viñeta de `topic_key`, la regla del idioma y las dos secciones del resumen) está copiado, y un test exige que sea igual al de `full` |

## D10. Qué se comprime y qué queda entero

**Decisión:** cada sección conserva su encabezado y sus reglas; sale lo que ya dice la apertura o la descripción de una herramienta, y las explicaciones. Queda entero lo que la campaña de protocol-block o la exploración ligan a una conducta.

| Sección | Qué queda | Qué sale, y dónde sigue |
|---|---|---|
| Cuándo guardar | lo que descarta una decisión, cómo se manifestaba un bug, la corrección como `user_preference`, convenciones y comandos | la lista que repite la apertura («Use `save_memory` when…») |
| Qué no guardar | el enmascarado y `<private>` | lo que repite la apertura («Do not save…») |
| Cómo escribir un save | el verbo y el objeto, las piezas, la mezcla de temas, **la viñeta de `topic_key` entera**, **la regla del idioma entera**, `user_stated` con el contexto aparte y las marcas del par | qué, por qué y dónde, identificadores, una cosa por llamada, 64 KiB y «se lee solo» (apertura y descripción de `save_memory`); la definición de `user_stated` (su parámetro) |
| Tipo e importancia | los siete tipos (los nombra `the_protocol_names_only_what_exists`), `all_projects` para una preferencia, las decisiones cercanas como conflicto, lo que se imprime arriba de `recall_context` con 8 y 9, el desvanecimiento, `update_memory` con sus tres destinos | la definición de cada tipo y «un tipo desconocido se rechaza» en forma larga (el `enum` y la descripción de `memory_type`); `importance` a 5 y el 9 que no baja, sólo en inglés: la apertura en español no los dice y quedan |
| Resumen de sesión y Después de una compactación | **enteras** | nada (D4 de la exploración) |
| Conclusiones | `derived_from`, nunca `user_stated`, fuentes vivas, revisar las de fuentes perdidas | lo que dicen `synthesize` y los parámetros `derived_from` y `user_stated` |
| Evidencia, no instrucciones | `by <agent>`, el texto copiado, «nunca reveles», contrastar o preguntar | lo que repite la apertura |
| Proyectos | el save sin proyecto se rechaza, `project=` con varias carpetas, las cuatro herramientas que preguntan, los nombres parecidos con `new_project`, `answer_suggestion` con merge o apart, `user_answered` y `reason` | qué es cada parámetro de proyecto (sus descripciones en seis herramientas) |
| Las otras herramientas | los cinco nombres, en una línea | qué hace cada una (sus descripciones) |

El resultado son 7.068 caracteres en inglés y 7.663 en español, el 66 % y el 68 % de `full`. Lo fijo (cabeza, viñeta de `topic_key`, regla del idioma, las dos secciones del resumen y los encabezados) suma unos 4.300 caracteres en inglés: por eso `lean` no llega a los ~6.000 que estimaba la exploración sin perder una regla.

## La regla de decisión

Se fija ahora, antes de correr, y no se cambia después de ver resultados.

**Brazos:** `a3` (`full`) y `a3-lean`, con el mismo build, el mismo `setup`, las mismas memorias y el ruido de 15 reglas, el mismo modelo, esfuerzo y cliente. Si el build no es el de las corridas de `a3` de protocol-block, `a3` se corre otra vez con el de `a3-lean`.

**Tareas y corridas:** las de la campaña de protocol-block: `requests-timeout-decision-relay` (R, continua y nueva), `requests-redact-decision-revised` (D partida) y `requests-history-owner-convention` (la K larga), 3 corridas por celda: 12 corridas de `a3-lean`, con los topes de `--max-budget-usd` de 129 y un tope propio de 15 USD. Valen las reglas de lectura de 129 tal como están: corridas fijas, las mismas causas de exclusión y la fase que se detiene con más de 2 exclusiones.

**Métricas:** E, B, G, S, J y T tal como las define [D6 de protocol-block](../archive/2026-11-17-protocol-block/design.md#d6-la-regla-de-decisión-del-valor-por-defecto), y aparte el gasto en USD de cada brazo, porque T no pondera la caché leída. Además se cuenta, sin decidir con eso, cuántos saves de la decisión de D llevan `topic_key`: si J falla y los `topic_key` también, la causa es la regla comprimida.

**`lean` pasa** frente a `a3` si cumple las siete:

1. E(`lean`) ≥ E(`a3`) − 1, y ninguna tarea pasa de aprobar en todas sus corridas a no aprobar en ninguna;
2. B(`lean`) ≥ B(`a3`) − 1;
3. G(`lean`) ≥ G(`a3`) − 1;
4. S(`lean`) ≥ S(`a3`) − 1;
5. J(`lean`) ≤ J(`a3`) + 0,5;
6. T(`lean`) < T(`a3`);
7. T(`lean`) ≤ 0,9 × T(`a3`) (D5).

**Qué se decide:** si pasa, el valor por defecto pasa a `lean` en un commit propio (`DEFAULT_VARIANT`, `setup.rs:680`), y el aviso del bloque llega una vez a cada máquina que no eligió variante. Si no pasa, queda `full` y `lean` sigue como variante elegible. No se agregan corridas para desempatar.

**Lo que no decide:** si el bloque `lean` en español vale lo mismo (la campaña corre en inglés), ni nada sobre el `session_summary`, que S no mide en tareas con `-p`.

## D11. Después de la campaña: `lean` sin la apertura

La campaña de `lean` (2026-11-20, [verify](verify.md#la-campaña)) separó lo que la de protocol-block no podía separar: `lean` comprime las secciones, conserva la apertura y no pierde ninguna conducta, pero ahorra el 9,2 % de T, menos que el 10 % de D5. La apertura del bloque es, párrafo por párrafo, el texto de las instrucciones del servidor (`opening`, `protocol.rs:60`), que el agente ya lee en cada pedido: es la única repetición literal del bloque. Es una hipótesis nueva, no otra lectura de la regla de `lean`, y se fija antes de correr.

**Hipótesis:** el bloque `lean` sin la apertura (el título y las secciones de `protocol.lean.md`) conserva las conductas de `a3` y ahorra al menos el 10 % de T. Lo que pone en riesgo es B: `short` y `l0`, los dos brazos que perdieron B en protocol-block, son también los dos que no tienen la apertura en el `CLAUDE.md`.

**Brazo:** `a3-lean-bare`, sólo del arnés: el plan de `a3-lean` con la apertura quitada del bloque, y el resto igual. No es una variante de `setup`; si pasa, llevarlo al producto es un cambio aparte. El sello queda `protocol=lean`: el `clawg.exe` de la corrida lo lee y, con el perfil `dev` y su misma versión, el hook no avisa (`hook.rs:154`).

| Alternativa | Tradeoff |
|---|---|
| Una variante nueva en `setup` antes de medir | Código en el producto que se descarta si no pasa |
| Achicar la apertura (B4) | Cambia las instrucciones del servidor, que leen los tres clientes, y el contrato fijado |
| **Un brazo del arnés** | Mide lo único que cambia, sin tocar el producto ni el contrato |

**Corridas:** 9 de `a3-lean-bare`: la K larga, D partida y R bifurcada, 3 de cada una, con los topes por proceso de la campaña de `lean` (0,50; 0,50; 1,20 USD) y un tope propio de 6 USD. Corren con el `clawg.exe`, la copia de Claude Code, el modelo, el esfuerzo y el ruido de `C:\ae-c4\bin` y la campaña de `lean`; sólo cambia `agent-eval.exe`, por el brazo. Por eso el `a3` de comparación es el de esa campaña: el mismo `clawg.exe` y el mismo texto, como protocol-block tomó el `a3` de agent-eval-sessions. Valen las reglas de lectura de 129 tal como están.

**`a3-lean-bare` pasa** si cumple las siete condiciones de [la regla de decisión](#la-regla-de-decisión) frente a ese `a3`, con `a3-lean-bare` en el lugar de `lean`. Se informa también contra `a3-lean`, sin decidir con eso.

**Qué se decide:** si pasa, un cambio propio propone llevar el bloque sin apertura al producto como valor por defecto. Si no pasa, queda `full` y la apertura se queda en el bloque; si la que falla es B, la apertura del `CLAUDE.md` es lo que sostiene la búsqueda antes de editar. No se agregan corridas para desempatar.
