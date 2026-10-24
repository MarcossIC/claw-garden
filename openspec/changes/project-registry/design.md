# project-registry — Diseño

Como la especificación, este diseño crece con cada entrega. Por ahora resuelve la **entrega 0** y la decisión abierta de la propuesta que le toca: [«El parche de la entrega 0 y la vuelta atrás»](proposal.md#abierto-para-el-diseño). Las demás quedan para la entrega que las necesita.

## D1. La huella valida la fila, en una función pura

`settle(rows: &[Vec<String>], remote: Option<&str>) -> Settled` (`crates/clawg-core/src/memory/project.rs`) recibe los remotos atados a cada fila del camino, de la más vieja a la más nueva, y la clave actual, y devuelve `Row(i)`, `Fresh` o `Contradicted` (RG1). No toca la base: `path_rows_in` lee las filas con sus remotos en una consulta (`json_group_array` sobre `project_remote`), y `settle_root_in` junta las dos cosas y busca el candidato libre cuando hace falta. Las tres entradas que antes repetían la regla (`root_id_in`, `project_id_in`, `plan_ids_in`) pasan por `settle_root_in`, así que no pueden divergir.

| Alternativa | Por qué no |
|---|---|
| Comparar la clave con los remotos del **proyecto** de la fila, no de la fila | El proyecto de una raíz que se unió a un clone es el de otra fila, y nada guarda de qué raíz vino cada remoto. Los remotos atados al id de la fila son lo único que el store sabe sin adivinar. El costo: una raíz que entró a un proyecto ajeno por su remoto no tiene remotos propios, así que no se contradice; pero su id de raíz no tiene memorias (se guardan bajo el proyecto compartido), y lo que llegue a ese camino no hereda nada |
| Elegir la fila más vieja cuando hay varias (lo de antes) | Es justo el fundido |
| El siguiente candidato libre cuando hay contradicción, sin mirar las demás filas | El candidato 1 queda tomado por la fila nueva y el bucle daría el 2 en el registro siguiente: un proyecto nuevo por sesión (lo advierte la propuesta) |
| Leer el primer commit ya en esta entrega | Es de la entrega 1, con su lectura fuera del camino caliente de los hooks |

Una fila sin remotos atados acepta la clave actual como su primer remoto. Es el límite que la propuesta acepta para esta entrega: si el repositorio anterior no tenía `origin`, el que llega con uno se queda con su fila. Lo cierra el primer commit en la entrega 1.

## D2. Los ids viejos de un camino compartido

Los ids viejos (`legacy_ids`) los deriva el cliente del camino crudo, sin saber qué repositorio guardó bajo ellos. Con un solo repositorio por camino eso daba igual; con varios, no: el recién llegado se llevaría lo que el anterior guardó desde una subcarpeta antes de que se resolvieran las raíces. `RootId::owns_legacy` es verdadero sólo para un camino nuevo y para la fila más vieja (RG2). Además, un id viejo que es el id de cualquier otra fila de `project` se deja, aunque sea del mismo camino; antes se comparaba sólo el camino.

| Alternativa | Por qué no |
|---|---|
| Mover los ids viejos al proyecto de cualquier fila elegida | Un segundo registro del recién llegado, que ya elige su propia fila, se llevaría las filas del anterior |
| Rechazar el registro cuando hay ids viejos con filas en un camino compartido | Bloquea la sesión de un repositorio que no hizo nada mal; dejar las filas donde están no pierde nada y `projects move` las lleva si hace falta |

## D3. El cliente sin `assigned` lee la clave, pero no la ata

El handler de `POST /v1/projects` leía `origin_key` sólo con `assigned`. Ahora la lee siempre, en un hilo bloqueante como antes, y `Registration::remote` la lleva; `write_in` sigue atando el remoto sólo con `assigned` (R18). Con ella, `plan_in` aplica la misma regla y responde 409 cuando la fila elegida no tiene el id derivado (RG4). Un puente abierto antes de la actualización ya entiende ese 409 y deja de usar el proyecto, en vez de escribir en el ajeno.

## D4. La regla de `doctor` separa los hechos del juicio

`inspect` suma `remotes: Vec<RemoteRow { remote, project_id }>`, leída del store como lo demás, sin texto de filas. La clave actual de cada raíz la lee `doctor::run` con `origin_key`, que sólo abre el `config` de git: el store sigue sin abrirse para escribir y ninguna ruta del usuario se modifica. `checks::project_remotes` es pura: agrupa por proyecto y da un `warn` por cada uno con dos o más remotos.

Dos remotos en un proyecto no prueban un fundido: un remoto renombrado y registrado con un clawg anterior deja lo mismo. Por eso es `warn` sin paso siguiente, y la evidencia nombra cada raíz con su clave actual para que el usuario vea cuál es cuál. Separar lo fundido queda fuera de alcance (la propuesta).

## D5. El parche no lleva migración

Es la decisión abierta de la propuesta. Una migración mínima haría que un binario anterior rechace el store (`SchemaNewer`) y no pudiera repetir el fundido, pero:

- tomaría un número de esquema que no agrega nada y un snapshot `pre-migration` en cada store al actualizar;
- `clawg update` ya restaura `pre-migration` cuando el binario viejo rechaza el store, y con una migración vacía esa vuelta atrás tiraría lo guardado después;
- lo que haría el binario anterior es recuperable: la segunda fila del camino no la ve (resuelve a la más vieja), y si el remoto del repositorio nuevo ya está en `project_remote`, une a ese proyecto las filas del id de raíz viejo con un snapshot `pre-rekey` y un movimiento en `memory_move`, que `clawg projects move --undo N` devuelve; el log del daemon nombra el número.

Así que se documenta: volver a una versión anterior a 0.62.1 en un store donde dos repositorios compartieron un camino puede volver a unirlos, y el `--undo` del movimiento que nombra el log lo deshace.

## Volver atrás

No hay esquema nuevo. Las filas que agrega la entrega son filas de `project` y de `project_remote` como las de antes; un binario anterior las lee (D5).
