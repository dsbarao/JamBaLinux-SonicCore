# Ponte local Claude Pro para Codex

`tools/claude_pro_mcp.py` disponibiliza uma ferramenta MCP local chamada
`claude_pro_audit`. Ela inicia o `claude --print` autenticado pela sessão OAuth
local do Claude Code e retorna o relatório em JSON ao Codex. Não usa
`ANTHROPIC_API_KEY` e se recusa a executar quando essa variável estiver
definida.

Por segurança, esta primeira versão é deliberadamente **somente leitura**:
ela permite revisão, diagnóstico e validação independente, mas nega edição,
instalação, commits, rede e comandos de mudança de estado. Isso fornece uma
segunda opinião rastreável sem permitir que um subprocesso autônomo altere o
projeto silenciosamente.

## Instalação

Com o login Pro ativo no Claude Code, execute:

```bash
/home/daniel/Projetos/JamBaLinux/tools/install-claude-pro-mcp.sh
```

Reinicie o Codex Desktop depois. Na nova sessão, peça uma auditoria concreta,
por exemplo: "use `claude_pro_audit` para checar se o equalizador altera o
PipeWire sem reiniciar o serviço".

O uso e os limites seguem a assinatura Claude Pro conectada à CLI local. A
ponte não garante acesso ilimitado nem converte o Claude em agente nativo do
Codex; ela apenas expõe uma ferramenta local auditável.

Para remover o registro:

```bash
codex mcp remove claude-pro-jambalinux
```

## Antigravity

O mesmo padrão existe para o Antigravity CLI local, em
`tools/antigravity_mcp.py`. Ele usa a sessão do keyring do `agy`, exige que
`GEMINI_API_KEY` esteja ausente e expõe `antigravity_audit` ao Codex. Instale
com:

```bash
/home/daniel/Projetos/JamBaLinux/tools/install-antigravity-mcp.sh
```

O modo headless oficial do Antigravity envia uma solicitação com `agy -p` e
retorna JSON. Nesta ponte ele é limitado a `--mode plan --sandbox`, para que
uma auditoria não vire uma edição autônoma do projeto.
