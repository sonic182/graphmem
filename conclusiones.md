# Conclusiones de las evaluaciones de recuperación

Pruebas del 2026-09-16 en la rama `feat/batched-eager-embeddings-and-retrieval-eval`.

## Configuración

- **Scripts:** `scripts/eval_retrieval.py`, con los grafos extraídos por
  `scripts/extract_eval_graphs.py`.
- **Datos:** HotpotQA (distractor), 2WikiMultihopQA y MuSiQue, validación. Se
  usan las primeras 50 preguntas de cada uno, salvo donde se indica.
- **Corpus:** `shared` salvo donde se indica. Todos los párrafos de las
  preguntas, sin duplicados, en un único scope; cada pregunta busca sobre todo
  el corpus.
  - HotpotQA: 491 párrafos.
  - 2Wiki: 446.
  - MuSiQue: 828.
- **Modelo:** `sentence-transformers/msmarco-distilbert-cos-v5` en CPU (AMD
  Ryzen 5 3550H, 8 hilos), binario release, batch 1.
- **Métricas:** media sobre las preguntas.
  - `recall@k`: fracción de los párrafos de apoyo que aparecen en el top k.
  - `MRR`: 1 / posición del primer párrafo de apoyo.
- **Precisión:** con 50 preguntas, cada punto de recall equivale a 1–2
  preguntas. Las diferencias de ±0.03 no son fiables.

Modos:
- `embeddings`: `recall` con `use_embeddings=true` (semillas semánticas +
  Personalized PageRank).
- `fts-raw`: `use_embeddings=false` con la pregunta tal cual.
- `fts-or`: `use_embeddings=false` con las palabras unidas con `OR`, como
  línea base BM25 justa.

Grafos:
- `none`: sin entidades.
- `mentions`: una entidad por título, más aristas cuando un párrafo menciona
  el título de otro de su misma pregunta.
- `spacy`: entidades y tripletas sujeto-verbo-objeto extraídas con spaCy
  `en_core_web_sm`, sin LLM.
- `oracle`: las tripletas del propio dataset. Solo cubren el camino de la
  respuesta, así que filtran la respuesta y solo sirven como techo.

## 1. Búsqueda léxica con preguntas completas

Antes, FTS5 exigía que aparecieran **todas** las palabras de la consulta, así
que una pregunta completa no encontraba nada.

Cambio en `src/infrastructure/sqlite.rs`:
- **Lenguaje natural:** las palabras se unen con `OR` y BM25 ordena. Las
  frases entre comillas y los `prefijo*` se conservan, y la demás puntuación
  separa palabras.
- **FTS5 estricto:** solo cuando aparecen `AND`, `OR`, `NOT` o `NEAR` en
  mayúsculas.

| dataset | `fts-raw` antes (recall@2 / @5 / @10 / MRR) | `fts-raw` ahora | `fts-or` |
|---|---|---|---|
| HotpotQA | 0 / 0 / 0 / 0 | 0.510 / 0.790 / 0.920 / 0.853 | 0.520 / 0.790 / 0.920 / 0.867 |
| 2Wiki | 0 / 0 / 0 / 0 | 0.585 / 0.730 / 0.765 / 0.970 | 0.585 / 0.730 / 0.765 / 0.970 |
| MuSiQue | 0 / 0 / 0 / 0 | 0.480 / 0.540 / 0.610 / 0.850 | 0.480 / 0.540 / 0.610 / 0.850 |

**Conclusión:** el agente ya puede usar `use_embeddings=false` con lenguaje
natural. En HotpotQA queda una diferencia mínima porque ahora se mantienen las
frases entre comillas.

## 2. Embeddings frente a BM25, sin grafo

| dataset | modo | recall@2 | recall@5 | recall@10 | MRR |
|---|---|---|---|---|---|
| HotpotQA | embeddings | 0.620 | 0.790 | 0.850 | 0.917 |
| HotpotQA | fts-or | 0.520 | 0.790 | 0.920 | 0.867 |
| 2Wiki | embeddings | 0.555 | 0.680 | 0.705 | 0.927 |
| 2Wiki | fts-or | 0.585 | 0.730 | 0.765 | 0.970 |
| MuSiQue | embeddings | 0.520 | 0.630 | 0.700 | 0.912 |
| MuSiQue | fts-or | 0.480 | 0.540 | 0.610 | 0.850 |

**Conclusión:** sin grafo, los embeddings no superan de forma consistente a
BM25. Ganan en HotpotQA (recall@2, MRR) y en MuSiQue (todo el rango); en
2Wiki pierden en todo. En preguntas llenas de nombres propios,
`msmarco-distilbert` aporta poco por sí solo, y el resultado depende del
dataset.

## 3. Efecto del grafo (embeddings)

| dataset | grafo | entidades | aristas | recall@2 | recall@5 | recall@10 | MRR |
|---|---|---|---|---|---|---|---|
| HotpotQA | none | 0 | 0 | 0.620 | 0.790 | 0.850 | 0.917 |
| HotpotQA | mentions | 491 | 317 | 0.650 | 0.840 | 0.930 | **0.937** |
| HotpotQA | spacy sin filtrar | 4496 | 488 | 0.630 | 0.760 | 0.890 | 0.912 |
| HotpotQA | **spacy filtrado** | 830 | 50 | **0.670** | 0.840 | 0.930 | 0.930 |
| 2Wiki | none | 0 | 0 | 0.555 | 0.680 | 0.705 | 0.927 |
| 2Wiki | mentions | 446 | 144 | **0.700** | **0.910** | 0.965 | 0.937 |
| 2Wiki | spacy filtrado | 670 | 39 | 0.660 | 0.825 | 0.935 | 0.960 |
| 2Wiki | oracle (techo) | 188 | 119 | 0.775 | 0.970 | **0.985** | **0.970** |
| MuSiQue | none | 0 | 0 | 0.520 | 0.630 | 0.700 | 0.912 |
| MuSiQue | mentions | 805 | 119 | 0.620 | 0.690 | 0.730 | 0.942 |
| MuSiQue | **spacy filtrado** | 1211 | 55 | **0.700** | **0.820** | **0.910** | 0.940 |
| MuSiQue | oracle (techo) | 152 | 100 | 0.930 | 1.000 | 1.000 | **1.000** |

**Conclusiones:**
- **El grafo aporta mucho en preguntas multi-salto, en los tres datasets.**
  En 2Wiki, recall@10 pasa de 0.705 a 0.935–0.965; en MuSiQue, de 0.700 a
  0.730–0.910. Sin grafo, el segundo párrafo necesario casi nunca aparecía;
  PageRank lo alcanza a través de las entidades. Esto valida el enfoque
  HippoRAG de gmem.
- **La calidad del grafo importa más que su tamaño.** El grafo spaCy sin
  filtrar empeoró recall@5 respecto a no usar grafo en HotpotQA. Filtrado,
  mejora claramente en los tres datasets; en MuSiQue incluso supera a
  `mentions` en todo el rango, aunque `mentions` no usa los títulos de cada
  pregunta para enlazar tan bien como en HotpotQA/2Wiki.
- **Con grafo, los embeddings sí superan a BM25** en los tres datasets. En
  2Wiki, recall@10 es 0.935 frente a 0.765; en MuSiQue, 0.910 frente a 0.610.
- **`oracle` confirma el techo:** con las tripletas del propio dataset (que
  filtran la respuesta), 2Wiki y MuSiQue llegan a recall@10 ≥ 0.985 y MRR
  ≥ 0.970. La brecha entre `spacy filtrado` y `oracle` (2Wiki: 0.935 vs 0.985;
  MuSiQue: 0.910 vs 1.000) mide cuánto queda por mejorar en la extracción de
  relaciones sin LLM.

### Filtrado del grafo spaCy

Análisis del grafo sin filtrar de HotpotQA (3669 entidades, tras quitar
fechas y nacionalidades):

| párrafos en los que aparece la entidad | entidades |
|---|---|
| 1 | 3247 (88 %) |
| 2 | 233 |
| 3 | 86 |
| 4 | 37 |
| 5 o más | 66 |

- Las entidades que no son títulos y aparecen en un solo párrafo eran 2839 de
  3181. No conectan nada: solo añaden semillas ruidosas a PageRank y tiempo de
  embeddings.
- Las más frecuentes eran genéricas: `england`, `italy`, `france`,
  `university`…

Filtros aplicados:
1. **En la extracción:** se descartan las etiquetas `DATE` y `NORP`
   (nacionalidades), además de las numéricas.
2. **Nombres canónicos:** un nombre que coincide con un título del corpus, o
   con su forma sin sufijo, se unifica con ese título. Por ejemplo, "Doctor
   Strange" pasa a ser "Doctor Strange (2016 film)", el equivalente a un
   hipervínculo.
3. **Tope de frecuencia:** fuera las entidades que no son títulos y aparecen en
   más del 2 % de los párrafos (`--spacy-max-df 0.02`).
4. **Mínimo de frecuencia:** fuera las entidades que no son títulos y aparecen
   en menos de 2 párrafos (`--spacy-min-df 2`).

Resultado en HotpotQA: de 4496 a 830 entidades, y el embebido bajó de 464 s a
253 s.

Pendiente: spaCy extrae muy pocas relaciones (menos de 1 por párrafo). La
conexión entre párrafos viene sobre todo de las entidades compartidas.

## 4. Corpus por pregunta frente a compartido

HotpotQA, primeras 10 preguntas, embeddings, sin grafo:

| corpus | párrafos por búsqueda | recall@2 | recall@5 | recall@10 | MRR |
|---|---|---|---|---|---|
| per-question | 10 | 0.650 | 0.850 | 1.000 | 1.000 |
| shared | 100 | 0.650 | 0.800 | 0.950 | 1.000 |

**Conclusión:** con el corpus por pregunta, recall@10 es 1.0 casi siempre y no
distingue nada. Hay que evaluar con el corpus compartido, como HippoRAG.

## 5. Rendimiento en CPU

`gmem reembed` sobre los mismos 100 párrafos de HotpotQA:

| batch | tiempo |
|---|---|
| 1 | ~40 s |
| 8 | 50.4 s |
| 32 | 78.8 s |

- **Batch 1:** 39.9 s, medido con esas mismas 100 memorias en la ejecución
  de 10 preguntas con corpus compartido (sección 4), no con `bench.py`.
- **Batches grandes:** cada grupo se rellena hasta el texto más largo, y en CPU
  eso cuesta más de lo que ahorra el batch.
- **Default actual:** batch 1 en CPU y 16 en CUDA (sin medir, porque no hay
  GPU).

Coste por paso de embeddings, con 50 preguntas y batch 1:

| paso | tiempo |
|---|---|
| HotpotQA sin grafo (491 memorias) | 176 s |
| HotpotQA `mentions` | 217 s |
| HotpotQA spacy sin filtrar | 464 s |
| HotpotQA spacy filtrado | 253 s |
| 2Wiki sin grafo (446 memorias) | 116 s |
| 2Wiki `mentions` | 157 s |
| 2Wiki spacy filtrado | 176 s |

**Instrucciones de CPU:** la CPU tiene AVX, AVX2, FMA y F16C, pero `gmem` se
compila para el x86-64 genérico (solo SSE2).
- La multiplicación de matrices de candle (`gemm`) detecta AVX2/FMA al
  arrancar y las usa.
- Las operaciones elemento a elemento y los kernels SIMD propios de candle no.
  Estos están condicionados a `target_feature = "avx2"` en tiempo de
  compilación.
- Pendiente medir `RUSTFLAGS="-C target-cpu=native"` (o `x86-64-v3`).

## 6. WGPU (iGPU Radeon Vega, rama `feat/wgpu_check`)

Backend experimental sobre el fork de candle con WGPU (`Device::new_wgpu(0)`,
backend explícito, `auto` sigue siendo CUDA → CPU). Mismos 100 párrafos de
HotpotQA que la sección 5, binario release compilado con
`CARGO_TARGET_DIR=target/wgpu cargo build --release --features wgpu`.

| backend | batch | tiempo |
|---|---|---|
| CPU | 1 | 41.6 s |
| WGPU | 1 | 11.2 s (3.7× más rápido; `gpu_busy_percent` 90–94 %) |
| WGPU | 4 | 11.4 s |
| WGPU | 8 | 11.7 s |
| WGPU | 16 | 11.6 s |

**El batch no cambió el tiempo total a partir de 4.** Con un corpus tan
pequeño (100 párrafos, un proceso por medición), la carga del modelo y el
arranque del proceso dominan sobre el cómputo del batch; hace falta un
corpus mayor para que el batch se note.

**Pánico con batch ≥ 2 y textos largos, ya resuelto:** el fork usa
`enqueue_64` para casi todas las operaciones (softmax, máscara, matmul), que
solo reparte el trabajo en el eje X del dispatch (tope 65535 grupos de 64).
La matriz de atención `(batch, heads, seq, seq)` lo supera con batch alto y
párrafos de 512 tokens. Se corrigió en `src/infrastructure/embedding.rs` sin
tocar el fork: `embed_documents` tokeniza una vez y agrupa por
`batch × seq² × n_heads ≤ 65535 × 64` (n_heads leído del `config.json` del
modelo), además del tope de `batch_size`. En CPU y CUDA no cambia nada,
porque ahí no existe el límite.

**Calidad igual que en CPU**, con corpus por pregunta (10 preguntas,
HotpotQA, sin grafo): recall@2 = 0.650, recall@5 = 0.850, recall@10 = 1.000,
MRR = 1.000 — igual que la sección 4.

**Prueba completa, 50 preguntas, corpus compartido (491 párrafos), sin
grafo** — la misma que la sección 2, ahora en WGPU:

| backend | tiempo de reembedido | recall@2 | recall@5 | recall@10 | MRR |
|---|---|---|---|---|---|
| CPU | 176 s | 0.620 | 0.790 | 0.850 | 0.917 |
| WGPU | 55.7 s (3.16× más rápido) | 0.620 | 0.790 | 0.850 | 0.917 |

Recall y MRR idénticos a CPU con el corpus grande, y sin pánico pese a que
este corpus sí tiene párrafos de 512 tokens con el batching por presupuesto
de tokens activo.

**Reembedido en los tres datasets** (50 preguntas, corpus compartido, todos
los grafos de la sección 3), comparado con la CPU cuando hay línea base
(sección 5):

| dataset | grafo | CPU (batch 1) | WGPU | aceleración |
|---|---|---|---|---|
| HotpotQA | none | 176 s | 55.8 s | 3.15× |
| HotpotQA | mentions | 217 s | 64.3 s | 3.37× |
| HotpotQA | spacy filtrado | 253 s | 63.4 s | 3.99× |
| 2Wiki | none | 116 s | 40.6 s | 2.86× |
| 2Wiki | mentions | 157 s | 46.5 s | 3.38× |
| 2Wiki | spacy filtrado | 176 s | 47.0 s | 3.74× |
| 2Wiki | oracle | — (no medido) | 43.6 s | — |
| MuSiQue | none | — (no medido) | 74.4 s | — |
| MuSiQue | mentions | — (no medido) | 82.5 s | — |
| MuSiQue | spacy filtrado | — (no medido) | 84.2 s | — |
| MuSiQue | oracle | — (no medido) | 76.6 s | — |

**Conclusión:** la aceleración crece con el tamaño del grafo (más
entidades/aristas para embeber): de ~3× sin grafo a ~4× con spaCy filtrado.
Todas las combinaciones dieron exactamente el mismo recall/MRR que el
resultado de CPU equivalente (sección 3), así que el batching por
presupuesto de tokens no cambia la calidad, solo evita el pánico.

Pendiente: proponer `enqueue_64_big` (ya usado en `binary.rs` del fork) para
las operaciones que le faltan, en el PR #3379 de candle; medir CPU en
MuSiQue para tener la aceleración completa.

## Pendiente

- **Más preguntas:** repetir con 100 o más para reducir el ruido.
- **Reutilizar vectores:** no volver a embeber las memorias en cada grafo (hoy
  unos 0.36 s por memoria).
- **Parámetros de `[retrieval]`:** compararlos lado a lado (`damping`,
  `memory_seed_weight`…).
- **Pesos de las semillas:** probar semillas de entidad ponderadas por IDF
  dentro de gmem, en lugar del tope de frecuencia del script.
- **Relaciones de spaCy:** mejorar su extracción, o probar un modelo mayor.
- **Compilación:** medir `target-cpu=native`.
