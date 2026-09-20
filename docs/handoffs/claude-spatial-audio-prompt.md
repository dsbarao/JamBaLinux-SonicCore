# Prompt pronto para Claude — implementar surround/binaural

Copie a partir da linha abaixo e envie ao Claude no diretório do projeto.

---

Você está trabalhando no repositório JamBaLinux SonicCore em
`/home/daniel/Projetos/JamBaLinux`.

Quero que você avalie e, se for tecnicamente seguro, implemente o MVP de áudio
espacial/binaural aberto descrito em
`docs/handoffs/claude-spatial-audio-plan.md`.

Antes de alterar qualquer arquivo:

1. leia integralmente `AGENTS.md`, `README.md`, `docs/rebranding.md`,
   `docs/protocol/audio-processing.md`, EVT-035, EVT-036, `src/spatial.rs`,
   `src/pipewire.rs`, o instalador/desinstalador e o plano de handoff;
2. se existir o repositório irmão privado, leia apenas para contexto conforme
   `AGENTS.md`, sem copiar informações privadas;
3. execute `git status --short` e preserve todas as mudanças existentes;
4. inspecione a versão instalada de PipeWire e os exemplos locais
   `/usr/share/pipewire/filter-chain/sink-virtual-surround-7.1-hesuvi.conf` e
   `sink-virtual-surround-5.1-kemar.conf`;
5. confirme com evidência o que já existe e liste qualquer divergência do plano.

Regras obrigatórias:

- Não modifique USB/HID, VID/PID, allowlists, firmware ou protocolo.
- Não implemente nem use nomes/arquivos/algoritmos proprietários de DTS, JBL
  Quantum Spatial, Dolby ou equivalentes.
- Use nomes neutros; o único modo é `binaural-stereo`.
- Não baixe nem inclua HRIR automaticamente. Exija arquivo do usuário com
  manifesto, hash e licença/proveniência.
- Não altere a saída padrão, Chat ou microfone.
- Não reinicie PipeWire/WirePlumber para alternar o recurso.
- Não crie dois componentes disputando roteamento. O MVP é manual: o jogo deve
  selecionar o sink espacial explicitamente.
- Encadeie a saída estéreo espacial no sink existente
  `jambalinux-soniccore-game-equalizer`; o EQ continua apontando ao Game físico.
- Falhe de forma fechada diante de alvo ausente/ambíguo ou dataset inválido.
- Não faça alegações de paridade acústica com o software original.

Modo de trabalho:

1. Faça primeiro uma auditoria e apresente um plano curto com arquivos e
   invariantes. Se houver uma decisão que altere a topologia acima, pare e peça
   aprovação antes de implementar.
2. Implemente em fases pequenas: validação do dataset, renderizador puro,
   serviço/supervisor, status, CLI, widget e empacotamento.
3. Prefira funções puras e testes unitários. Gere uma HRIR sintética temporária
   para testes; nunca comite um arquivo proprietário.
4. Mantenha o backend espacial separado de `src/pipewire.rs` quando isso evitar
   regressões no equalizador, mas reutilize padrões comprovados de lock,
   persistência, descoberta e health check.
5. Atualize README, protocolo, instalador e desinstalador junto com o código.
6. Não instale nem teste no áudio real até os testes estáticos/unitários
   passarem e eu autorizar a validação na sessão de usuário.

Entregáveis mínimos:

- contrato validado para `manifest.json` + `hrir.wav` de 14 canais;
- configuração PipeWire determinística de entrada 7.1 e saída estéreo;
- serviço persistente desativado por padrão e sem nós quando desligado;
- `spatial status --format json` com saúde real do grafo;
- enable/disable reversível, sem alterar o default;
- integração `spatial -> equalizer -> Quantum Game`;
- widget com controle apenas depois do backend comprovado;
- não remover mensagens da interface nem implementar perfis personalizados do
  equalizador durante as fases 1–7; esses itens estão engatilhados no plano
  para uma rodada separada após a validação do áudio espacial;
- testes de canais, escaping, falhas, isolamento, rollback e reconnect;
- documentação das limitações e roteiro de aceitação real.

Critérios de revisão:

- Mostre o diff e explique a topologia antes da instalação.
- Rode `cargo fmt --check`, `cargo test --all-targets`, `git diff --check` e o
  linter QML disponível.
- Informe explicitamente o que foi medido e o que continua apenas simulado.
- Se não for possível garantir fallback, isolamento e ausência de disputa com
  o EQ, não ative o recurso: entregue somente a infraestrutura segura e relate
  o bloqueio.

Comece pela auditoria. Não assuma autorização para instalar pacotes, baixar
datasets, alterar serviços ativos ou reiniciar o desktop.

---
