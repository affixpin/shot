

<div align="center">
  <img src="shooter.jpeg" width="200" />
  <h1>agente shot</h1>
  <p><strong>agente mínimo, portátil y compatible con Unix</strong></p>
  <p>prueba la demo en vivo: <a href="./gateway">bot de telegram multi-tenant</a> → <a href="https://t.me/autoshot_bot">@autoshot_bot</a></p>
</div>

---

## motivación

Creé shot porque no podía encontrar un agente ligero para uso multi-tenant. Cada opción era o bien un framework enorme con runtime y plugins integrados, o una herramienta para construir tu propio framework enorme. Ninguno hacía lo único que yo quería: iniciarse, usar herramientas, guardar la sesión y salir.

Qué es shot:

- De corta duración. Lo inicias por mensaje, hace el trabajo y sale.
- Extensible. Las herramientas son archivos `.toml` simples que ejecutan comandos de shell. No hace falta código para añadir o quitar una.
- Fácil de integrar. `--json` te da un evento por línea en stdout, puedes hacer pipe a donde quieras.
- Pequeño. Binario estático de 4 MB, imagen Docker de 31 MB, alrededor de 3 MB de RAM mientras procesa un mensaje, y unos milisegundos para iniciarse en frío.
- Compatible con Unix. Lee de stdin, escribe en stdout, funciona bien con pipes y redirecciones.

Ese último punto es la verdadera razón de su existencia. Me gustan herramientas como `vim`, `fzf`, `fd`, `rg`, `jq`. Cada una hace una sola cosa y no se interpone. Shot intenta ser ese tipo de herramienta para ejecutar un agente LLM.

## inicio rápido

La forma más fácil de probar shot es con Docker.

### gemini

```bash
# simple question, no tools
docker run --rm \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY \
  affixpin/shot "what's the capital of France?"

# with the default tools (file I/O, shell, grep, etc.)
docker run --rm \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY \
  affixpin/shot --tools "list the files in /etc and summarize"

# with web search (free Jina key at https://jina.ai/reader)
docker run --rm \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY \
  affixpin/shot --tools \
  --tools.web_search.vars.jina_api_key=YOUR_JINA_KEY \
  "who is Dmytro Pintak?"
```

### anthropic

```bash
docker run --rm \
  -e SHOT_CONFIG_AGENT_PROVIDER=anthropic \
  -e SHOT_CONFIG_ANTHROPIC_API_KEY=YOUR_KEY \
  affixpin/shot --tools "what's in the current directory?"
```

### openai

```bash
docker run --rm \
  -e SHOT_CONFIG_AGENT_PROVIDER=openai \
  -e SHOT_CONFIG_OPENAI_API_KEY=YOUR_KEY \
  affixpin/shot --tools "what's in the current directory?"
```

### compilar desde el código fuente

Es un workspace de Rust, por lo que `rustup` es el único prerrequisito.

```bash
git clone https://github.com/affixpin/shot
cd shot

# install the binary to ~/.cargo/bin
cargo install --path shot

# or build without installing
cargo build --release -p shot
sudo ln -s "$(pwd)/target/release/shot" /usr/local/bin/shot
```

En la primera ejecución, shot coloca sus herramientas predeterminadas y el prompt "soul" en `~/.local/share/shot/`. No hay paso de configuración separado, solo establece la clave:

```bash
export SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY
shot "hello"
```

## configuración

La configuración se ensambla en cuatro capas. Las capas posteriores tienen prioridad.

1. Valores predeterminados compilados.
2. `~/.config/shot/agent.toml` (o `$XDG_CONFIG_HOME/shot/agent.toml`) si existe.
3. Variables de entorno: `SHOT_CONFIG_<SECTION>_<FIELD>=value`.
4. Flags de CLI: `--config.<section>.<field>=value`.

Algunos ejemplos:

```bash
# switch provider via env
SHOT_CONFIG_AGENT_PROVIDER=openai SHOT_CONFIG_OPENAI_API_KEY=... shot "..."

# override the model on a single run
shot --config.gemini.model=gemini-2.5-pro "explain quickly"

# persist the current setup to a file
shot --config.gemini.api_key=YOUR_KEY config show > ~/.config/shot/agent.toml
```

`shot config show` imprime la configuración combinada en formato TOML. Útil cuando una flag o variable de entorno no hace lo que esperas.

## herramientas

Las herramientas se almacenan en `~/.local/share/shot/tools/` como archivos `.toml`. Cada una describe un comando de shell y sus parámetros; el LLM ve la descripción, llama a la herramienta con argumentos, y shot ejecuta el comando con esos argumentos establecidos como variables de entorno.

Conjunto predeterminado, instalado en la primera ejecución:

| Herramienta       | Qué hace                                       |
|-------------------|------------------------------------------------|
| `file_read`       | Leer un archivo                                |
| `file_write`      | Escribir un archivo                            |
| `file_remove`     | Eliminar un archivo o directorio               |
| `list_files`      | `ls -la`                                       |
| `search_text`     | `grep -rn` entre archivos                      |
| `shell`           | Ejecutar un comando de shell                   |
| `web_search`      | Búsqueda web vía Jina (requiere `jina_api_key`) |
| `web_read`        | Obtener una URL como markdown vía Jina         |
| `tg_send`         | Enviar un mensaje de Telegram                  |
| `memory_store`    | Guardar un dato vía `engram`                   |
| `memory_recall`   | Buscar en la memoria guardada vía `engram`     |

Para añadir las tuyas, coloca otro `.toml` en el directorio de herramientas. Shot lo detectará en la próxima ejecución.

## gateway

Shot es de ejecución única: iniciar, procesar un mensaje y salir. Para cualquier cosa de duración más larga (un bot de Telegram, un webhook, una app de Slack) necesitas un proceso host que decida cuándo iniciar shot y con qué argumentos. Eso es `shot-gateway` — un servidor Node.js mínimo distribuido como una imagen separada.

La demo en vivo en [@autoshot_bot](https://t.me/autoshot_bot) se ejecuta sobre ella. Hace polling a Telegram, inicia un contenedor shot por mensaje con aislamiento por chat, y proxya las llamadas LLM + Jina para que las claves API nunca entren en un contenedor shot.

```bash
docker run -d --name shot-gateway --restart=always \
  -e TELEGRAM_TOKEN=... \
  -e GEMINI_API_KEY=... \
  -e JINA_API_KEY=...                                  # optional, enables web_search/web_read
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /opt/shot-data:/data \
  affixpin/shot-gateway:latest
```

La configuración completa (scripts de despliegue en GCP, conexión con secret manager, explicación del proxy) se encuentra en [`./gateway`](./gateway).
