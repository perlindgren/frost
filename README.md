# Workflow

- ollama
- qwen38:27b

## Settings 

- User settings `setting.json`
```
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
- Ollama, `cat  /etc/systemd/system/ollama.service.d/override.conf`
```
[Service]
Environment="HSA_OVERRIDE_GFX_VERSION=11.0.0" "OLLAMA_MAX_VRAM=24GiB" "OLLAMA_KEEP_ALIVE=-1" "OLLAMA_KV_CACHE_TYPE=q8_0"
```

## Tools

- ollama 2.0 as systemd daemon, to change parameters:
  ```sudo systemctl daemon-reload && sudo systemctl restart ollama```
- `nvtop` (to inspect memory and cpu usage)
- `ollama ps` (to validate memory distribution)
```
NAME           ID              SIZE     PROCESSOR    CONTEXT    UNTIL   
qwen3.8:27b    22130167c4c2    17 GB    100% GPU     98304      Forever
```
