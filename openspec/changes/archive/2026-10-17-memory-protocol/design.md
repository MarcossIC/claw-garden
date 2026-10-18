# memory-protocol — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas que se descartaron. Las alternativas de entrega (prompts, recursos, todo en las instrucciones) están en la [exploración](exploration.md#enfoques-posibles).

## D1. Dónde termina la apertura

**Elegido:** la apertura son los párrafos entre el título y la primera línea `## `. El protocolo se lee como un documento: la apertura es su resumen y cada sección desarrolla un punto. `fn opening(protocol: &str) -> String` es pura, y `get_info` la llama en cada pedido (`server.rs:1132`). El costo es unir unas veinte líneas, frente a un `get_info` que se pide una vez por sesión.

| Alternativa | Por qué no |
|---|---|
| Marcas HTML (`<!-- instructions -->`) | Una marca mal cerrada manda texto de más sin que nadie lo vea, y la estructura de títulos ya separa lo mismo |
| Dos archivos (corto y largo) | Lo que dice uno lo repite el otro, y las dos copias se desfasan |
| Un `LazyLock<String>` | Agrega estado global para ahorrar una operación de microsegundos en un pedido que llega una vez |
| Mandar la apertura con sus saltos de línea | El cliente la muestra como prosa: un salto en medio de una oración es ruido en cada sesión, y el tope de R2 se gastaría en saltos |

El texto completo (`PROTOCOL`) queda en el binario y hoy sólo lo leen los tests de R3 y R4. Lo van a entregar el hook de sesión (25) y el setup (26). Hasta entonces, la apertura es el único texto que llega solo al agente, y por eso lleva los hábitos que más rinden (R3).

## D2. El enum del schema sin cambiar el tipo

**Elegido:** `memory_type` sigue siendo `Option<String>` en `SaveMemoryArgs` y en `UpdateMemoryArgs`. El schema suma `enum` con `#[schemars(transform = memory_type_enum)]`, una función que lee `MemoryType::ALL` y agrega `null` (el `default` declarado es `null`, `agent_contract.json`).

| Alternativa | Por qué no |
|---|---|
| Un `enum` de Rust con `Deserialize` | Un valor desconocido falla al deserializar `Parameters<…>` y llega al modelo como un error de protocolo, cuando la regla del puente es contestar cada falla con una oración (`server.rs:8-14`) |
| `#[schemars(extend("enum" = [...]))]` | Repite la lista dentro del atributo, una quinta copia |
| `schema_with` | Reemplaza el schema entero: hay que reconstruir `type` y `default` a mano y no se ve si `schemars` sigue sumando la descripción del comentario |

El enum no le quita al cliente la posibilidad de mandar otra cadena: la validación de verdad es la de D3.

## D3. Dónde se valida el tipo

**Elegido:** en el handler de `POST /v1/memories` (`api.rs:768`), después de comprobar `content` y antes de `check_operation_id` y del outbox. Con el mismo mensaje que `PATCH`: `PatchError::UnknownType` envuelve el error nuevo de `MemoryType`.

| Alternativa | Por qué no |
|---|---|
| En el worker (`store_insert.rs:141-145`) | Para cuando corre, el daemon ya contestó `201`/`202` a muchos saves y un rechazo no llega al que llamó. Además, una fila del outbox escrita por una versión anterior fallaría al reproducirse, y la base perdería un save que ya se había aceptado |
| En el puente | Sólo cubre a los clientes MCP: un `POST` directo seguiría cayendo en `semantic_fact`. La regla tiene que estar donde está la de `PATCH` |
| En los dos | Dos mensajes que se desfasan, para un caso que el daemon ya resuelve |

El reemplazo por `semantic_fact` del worker se queda para el outbox viejo.

## D4. La lista de tipos en el núcleo

**Elegido:** `pub const ALL: [MemoryType; 7]` y `as_str` públicos en `MemoryType` (`store_insert.rs:20-41`), más `impl FromStr for MemoryType` con un error `UnknownMemoryType(String)` (`thiserror`) cuyo mensaje se arma con `ALL`. `from_db_str` queda como está, porque lo usan las lecturas de la base, donde un valor desconocido es `None` y no un error.

Un test fija que `ALL` y `from_db_str` aceptan lo mismo. Al agregar un octavo tipo, el `match` de `is_claim_bearing` ya obliga a tocar el archivo (`store_insert.rs:43-51`), y ese test obliga a sumarlo a `ALL`.

## D5. Qué cuenta como un nombre en el protocolo

**Elegido:** un texto entre comillas invertidas que es una palabra en minúsculas, con o sin `_`. Eso cubre los nombres de herramientas, de propiedades y de tipos. Deja afuera los ejemplos y las marcas (`` `Fixed lexical bonus ignoring paths` ``, `` `<private>` ``, `` `piece=K/N` ``, `` `by <agent>` ``). Los nombres válidos se leen de `tool_router.list_all()`: los nombres de las herramientas y las claves de `properties` de cada `inputSchema`. A eso se suman los tipos de `MemoryType::ALL`.

| Alternativa | Por qué no |
|---|---|
| Sólo los que terminan en un nombre de herramienta | No ve un tipo mal escrito (`user_preferences`) ni un argumento renombrado |
| Comparar el texto con cada descripción | Frágil y no protege nada que importe ([exploración, Descartado](exploration.md#descartado)) |

## D6. El idioma

**Elegido:** inglés para el protocolo, para los saves y para las búsquedas, sin importar el idioma de la conversación. Las herramientas, las descripciones y las notas del servidor ya están en inglés. english-recall (21) midió que el cruce de idioma es lo que pierde las filas, y que con los dos lados en el mismo idioma la recuperación vuelve. Un idioma fijo es lo único que el agente puede cumplir sin saber cómo va a llegar la próxima consulta.

La regla vale para lo que se guarde de ahora en más. Lo que ya está guardado en español se queda así. El corte por pareja de `rag-eval` sigue mostrando el cruce en el golden actual, que no cambia con este cambio.
