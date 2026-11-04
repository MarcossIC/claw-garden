# preference-model — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 2 y 4 | M | session-start-context, que reparte el presupuesto | exploración; hay decisiones de la persona abiertas |

**Tesis:** el problema de las preferencias no es dónde se guardan, sino qué entra solo a cada sesión. Hoy decide un número que pone el agente, sin mirar quién lo dijo ni a qué proyecto corresponde. Hace falta decidir el alcance y el sello antes de instalar los hooks en todos los proyectos.

## ¿La tabla `memory` escala?

**Crece sin límite, y hoy nada la recorta.** Mantenimiento baja el tier y limpia vectores huérfanos, pero no borra filas (`run_maintenance_once_at`, `crates/clawg-core/src/memory/maintenance.rs:93`).

**Cuánto crece.** En prod hay 56.279 filas y 442 MB. 54.436 vienen de una importación de unas 3.800 notas, unas 14 filas por nota, porque cada nota larga se parte. Las propias son 1.843 en 22 días, casi todas de un solo proyecto, unas 84 por día. Con todos los proyectos activos, 100.000 filas o más en un año es un orden razonable, sin medir.

**Lo que dicen las mediciones:**
- El tamaño no cambia la pérdida de la búsqueda. 50.000 filas no pierden más que 10.000: decide la fracción de filas ajenas ([verify de knn-prefilter](../knn-prefilter/verify.md)).
- El costo sí crece: `knn_memory` da 175 ms de p50 en prod. Si eso molesta, lo decide knn-prefilter, no la forma de las tablas.

**Conclusión:** la tabla aguanta para esta etapa. Lo que sí falta es una política de olvido, que no bloquea la migración y queda como pregunta para después del corte.

## ¿Tabla aparte, o un tipo?

El tipo ya existe: `user_preference`, con 141 filas. No es un problema de volumen.

**Por qué no tablas por tipo.** Ya se probó: `episodic_trajectory` y `procedural_skill` son tablas por tipo, y [retire-legacy-kinds](../retire-legacy-kinds/exploration.md) las saca. Vacías, le cuestan a cada búsqueda dos KNN y dos consultas, y cada función que recorre filas tiene que acordarse de ellas.

**Lo que costaría una tabla nueva:** volver a escribir deduplicación, detector, enmascarado, exportación, importación con su deshacer, unión y borrado de proyectos.

**Recomendado:** se queda en `memory`, y se le agrega lo que le falta: alcance y sello.

## El alcance

**Hoy:** `fetch_auto_rows` filtra por agente y no por proyecto, a propósito: el comentario de `crates/clawg-core/src/memory/workspace.rs:348` dice que una preferencia describe a la persona. Pero 18 filas reales mezclan reglas generales con reglas de un solo repositorio, y las dos entran en todos.

**A. Por el proyecto de la fila (recomendado):** entra la fila del proyecto actual y la que no tiene proyecto. Una preferencia general se guarda sin proyecto.
- **A favor:** usa lo que hay y no agrega columnas.
- **En contra:** `save_memory` no tiene hoy un argumento para guardar sin proyecto desde un repositorio, y lo ya guardado hay que clasificarlo. Una preferencia general que el agente guarde mal no viaja. Cambia una decisión escrita en el código.

**B. Una columna de alcance propia (`persona` o `proyecto`):**
- **A favor:** dice exactamente qué es.
- **En contra:** una migración más, y hay que mantenerla junto a `project_id` y al `scope` que ya existe.

**C. Dejarlo como está y confiar en el tope de session-start-context:**
- **A favor:** no cuesta nada.
- **En contra:** las reglas de otro repositorio siguen entrando, sólo que menos.

## Entre agentes

El workspace sólo carga filas del agente (`specs/sharing`, R5). Cursor arranca sin las preferencias que se le dijeron a Claude Code, aunque describen a la misma persona. Las decisiones y procedimientos pueden seguir siendo del agente; las preferencias tendrían que seguir lo que el agente puede leer. Cambia R5.

## Importancia

Entra lo que tiene `importance >= 8` y tier 1. El número lo pone el agente, sin más control. Lo importado queda en 5 salvo lo fijado en origen, así que las 89 preferencias importadas no entran solas y sólo se alcanzan buscando.

## Envenenamiento

**El riesgo, verificado:** la consulta del workspace no mira `created_by_type` ni `source`. Una fila que el agente guardó como `user_preference` con importancia 8 o más, inferida o sacada de un texto que leyó, entra como «always active» en cada sesión y en cada proyecto, antes de cualquier consulta. El hook dice que es evidencia, pero el encabezado dice lo contrario.

**Lo que ya hay:** el enmascarado de secretos, los márgenes de `untrusted-recall` y `user_stated`, que deja `created_by_type = 'user'`. Es una declaración del agente, no una prueba.

**Recomendado:** sólo entra sola una preferencia con `created_by_type = 'user'`. Lo inferido se alcanza buscando, marcado como tal. Cuesta que una preferencia real sin declarar deje de entrar sola; cuántas son se cuenta antes de decidir.

## Cómo se mide

- Bytes de `session-start` por proyecto antes y después.
- Filas elegibles por alcance y por sello, contadas en prod con una lectura.
- Test: una fila inferida con importancia 9 no entra al workspace; una declarada, sí.
- Test: una preferencia de otro proyecto no entra; una sin proyecto, sí.

## Por qué va en la nube, y cuándo

La exploración necesita a la persona; la aplicación se prueba en Linux. Va después de session-start-context, con las decisiones tomadas.

## Decisiones abiertas (de la persona)

- [ ] Alcance: A, B o C. Recomendado: A.
- [ ] Si las preferencias cruzan agentes. Recomendado: sí.
- [ ] Sello: sólo lo declarado entra solo. Recomendado: sí, después de contar.
- [ ] Cómo se clasifican las preferencias que ya existen: revisión a mano de las que hoy entran, con la persona.
