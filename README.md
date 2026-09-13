# Workflows

- ollama
- qwen38:27b

---

## Ollama

### Settings 

- User settings `setting.json` (linux)
```json
  "default_profile": "write",
    "default_model": {
      "provider": "ollama",
      "model": "qwen3.8:27b",
      "enable_thinking": true,
      "effort": "medium",
    },
    "dock": "right",
    "favorite_models": [],
    "model_parameters": [],
  },
  "language_models": {
      "ollama": {
        "context_window": 98304,
        "api_url": "http://localhost:11434",
        "auto_discover": false,
        "available_models": [
          {
            "name": "qwen3.8:27b",
            "display_name": "qwen3.8:27b",
            "max_tokens": 98304,
            "supports_tools": true,
            "supports_thinking": true,
            "supports_images": true,
          }
        ]
      }
    },
```
- Ollama config: 
  - Linux with systemd control:
    `cat  /etc/systemd/system/ollama.service.d/override.conf`
    ```
    [Service]
    Environment="HSA_OVERRIDE_GFX_VERSION=11.0.0" "OLLAMA_MAX_VRAM=24GiB" "OLLAMA_KEEP_ALIVE=-1" "OLLAMA_KV_CACHE_TYPE=q8_0"
    ```
  - Win11:
    - none so far 

### Tools

- Ollama 2.0 as systemd daemon, to change parameters:
  `sudo systemctl daemon-reload && sudo systemctl restart ollama`
- `nvtop` (to inspect memory and cpu usage)
- Check the GPU/CPU mem partitioning:
  `ollama ps` (to validate memory distribution)
  ```
  NAME           ID              SIZE     PROCESSOR    CONTEXT    UNTIL   
  qwen3.8:27b    22130167c4c2    17 GB    100% GPU     98304      Forever
  ```
- Systemd Linux `journalctl -u ollama.service -f` (to follow the output of the ollama server)
- Win11 
  `Get-Content -Path "$env:LOCALAPPDATA\Ollama\server.log" -Tail 50 -Wait`

---

## LLAMA with DeepSeek

### Setup

- Install bare bones `llama`:
- Run in one terminal:
  ``` .\llama.exe serve -hf unsloth/Qwen3.8-27B-GGUF:UD-Q4_K_M --parallel 1 "-ngl" all "-fa" on "-c" 65536 "-ctk" q8_0 "-ctv" q8_0```

    - `-fa` flash attention (needed for context quantization)
    - `-ctx` q8_0 quantization 
    - `-c` context size
    - `-ctv` ...not sure
- Run in another terminal:
  ```npx @deepseek-ai/dsh web```

- Settings for dsh
```yaml
ui-onboarding:
  welcomeNoticeVersion: 2026-08-13.1
llm-pi-ai:
  providers:
    llama-local:
      apiKeyEnv: LLAMA_LOCAL_API_KEY
      api: openai-completions
      baseURL: http://127.0.0.1:8080
      models:
        - id: unsloth/Qwen3.8-27B-GGUF:UD-Q4_K_M
          name: unsloth/Qwen3.8-27B-GGUF:UD-Q4_K_M
          maxTokens: 65536
          contextWindow: 65536
# Provider Tuning Configuration
temperature: 0.15 # Drastically lowers hallucinations for precise syntax
top_p: 0.90 # Filters out low-probability logic steps
frequency_penalty: 0.0 # Ensures essential Rust structural syntax keywords are repeated cleanly
max_tokens: 65536 # Grants a large output context window per agent thought
agent-default-model:
  provider: llama-local
  model: unsloth/Qwen3.8-27B-GGUF:UD-Q4_K_M
```


## Examples

Run the examples with:

```shell
RUST_LOG=info cargo --example gizmo
```

or under Win11 with Powershell:

```powershell
$env:RUST_LOG="info"; cargo run --example gizmos
```
