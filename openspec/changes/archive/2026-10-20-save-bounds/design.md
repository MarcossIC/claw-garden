# save-bounds — Diseño

## D1. Un tope en bytes de 64 KiB, en el handler (R1)

**Elegido:** `MAX_CONTENT_BYTES = 65 536`, comparado con `content.len()` en una función pura, `check_content`, que reemplaza la comprobación de vacío de `save_memory` en el mismo lugar. Un `400` ahí es el último punto en que se puede decir que no sin perder nada: una vez en el outbox, el save sólo puede fallar después de la respuesta.

El valor sale de la medición de la [propuesta](proposal.md): a 64 KiB un save escribe unos 8 MB de progreso y ocupa medio segundo de hilo con el embedder falso, lo que la máquina absorbe; al doble, el costo se cuadruplica. La nota más larga que midió 44 (45 372 caracteres) entra con margen, también si tiene algo de texto no ASCII.

| Alternativa | Por qué no |
|---|---|
| Tope en pedazos | Exige correr el tokenizador en el handler, que no lo tiene, y el número depende del tokenizador: un mismo texto pasaría o no según la versión |
| 32 KiB | Rechaza la nota más larga de 44, y la medición no muestra un costo que lo justifique |
| Truncar en silencio | Guarda otro texto que el que el agente cree guardado |
| Partir el save en el handler | Cambia lo que es un save (un `operation_id`, un `topic_key`, un vínculo `memory_piece`) |
| Comprobar también en el puente | Cualquier cliente con el token lo saltea; el daemon tiene el contrato y su frase ya llega al modelo |

**Bytes de lo que llega, antes de enmascarar.** Enmascarar puede cambiar el largo, y el tope protege del costo que paga el daemon por lo que recibe; un texto sobre el tope no se enmascara ni se escribe.

**Lo que el tope no cubre:** el tiempo de los KNN del detector, uno por pedazo, crece con el store (la exploración estimó de 11 a 54 ms por escaneo entre 10 000 y 50 000 filas, es decir de 2,5 a 12 s para un save en el tope). Un save así puede contestar `202 unconfirmed` a los 10 s, que es cierto: sigue en el outbox. Bajar ese costo es de knn-prefilter (37), no de un tope más bajo.

## D2. El progreso queda como está

**Elegido:** sin cambio de formato ni migración. Con el tope, el peor caso es el medido (228 pedazos, 8 MB).

| Alternativa | Por qué no |
|---|---|
| B: progreso sin `context` repetido | Cada `UPDATE` reescribe la fila entera, `content` incluido (64 KiB por pedazo en el tope, unos 15 MB en total): adelgazar el JSON baja el total de unos 23 MB a unos 17, sigue siendo cuadrático y pone en riesgo la comprobación de la cabecera al reanudar |
| C: una tabla por pedazo (V18) | Lineal, pero es una migración con vuelta atrás y prueba en V17, un lugar más con texto del save que `purge` tendría que limpiar, y una numeración que coordinar con 51 y 56, para ganar medio segundo en el peor caso |

Si un cambio futuro sube el tope o mide un costo mayor con el modelo real, C es el camino.

## D3. Sin `DefaultBodyLimit`

El defecto de 2 MB del extractor `Json` ya acota lo que se lee y deserializa, y un tope propio del cuerpo tendría que elegirse para todas las rutas y contestaría un `413` sin la frase del daemon. No hay un uso que lo pida.
