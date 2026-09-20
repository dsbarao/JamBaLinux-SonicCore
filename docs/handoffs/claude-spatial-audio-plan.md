# Plano de implementação — áudio espacial/binaural aberto

## Objetivo

Implementar no JamBaLinux SonicCore um modo opcional que receba áudio 7.1 de
jogos e produza estéreo binaural para o JBL Quantum 810 usando convolução HRIR
no PipeWire. A implementação deve ser aberta, independente de fornecedor e
desativada por padrão.

Não é objetivo reproduzir, anunciar compatibilidade ou usar nomes, algoritmos,
arquivos ou respostas impulsivas proprietárias de DTS, JBL Quantum Spatial,
Dolby ou qualquer outro fornecedor.

## Estado confirmado do projeto

- O recurso espacial do software original é processamento no host; os eventos
  EVT-035 e EVT-036 não produziram tráfego USB.
- `src/spatial.rs` já contém perfil persistente, trava de mutação, modos
  `off`/`binaural-stereo` e um preflight somente leitura.
- O widget mostra o estado espacial, mas propositalmente não oferece controle.
- PipeWire e `libpipewire-module-filter-chain` estão disponíveis na máquina do
  mantenedor.
- O preflight atual falha somente porque não existe um dataset HRIR validado em
  `$XDG_CONFIG_HOME/jambalinux-soniccore/spatial/hrtf/`.
- O equalizador já possui serviço persistente, roteamento seguro e uma entrada
  estéreo virtual chamada `jambalinux-soniccore-game-equalizer`.
- O PipeWire instalado fornece como referência local
  `/usr/share/pipewire/filter-chain/sink-virtual-surround-7.1-hesuvi.conf`.

Documentação primária:

- [PipeWire filter-chain](https://pipewire.pages.freedesktop.org/pipewire/page_module_filter_chain.html)
- [pipewire-filter-chain.conf(5)](https://pipewire.pages.freedesktop.org/pipewire/page_man_pipewire-filter-chain_conf_5.html)
- `docs/protocol/audio-processing.md`
- `docs/protocol/experiments/EVT-035.md`
- `docs/protocol/experiments/EVT-036.md`

## Topologia do MVP

```text
Jogo configurado para 7.1
        │ FL FR FC LFE RL RR SL SR
        ▼
JamBaLinux Game Spatial (entrada virtual 7.1)
        │ 14 convolvers HRIR + mixers L/R
        ▼
estéreo binaural
        ▼
JamBaLinux Game Equalizer (entrada virtual estéreo existente)
        ▼
Quantum Game físico (PCM 0 confirmado)
```

O spatializer deve apontar sua saída estéreo para o equalizador existente. O
equalizador continua sendo o único componente que aponta para o dispositivo
físico. Isso mantém a ordem `spatial -> EQ -> headset`, preserva as curvas já
implementadas e evita duplicar a lógica sensível de descoberta do Game PCM.

Na primeira versão, o usuário seleciona **JamBaLinux Game Spatial** como saída
do jogo. O projeto não deve tornar a saída espacial padrão, mover aplicações
automaticamente ou competir com o roteador do equalizador. Alguns jogos só
negociam 7.1 ao iniciar; a interface deve avisar que pode ser necessário
selecionar a saída e reiniciar o áudio/jogo.

## Contrato do dataset

Suportar inicialmente um único formato, pequeno e verificável:

- arquivo WAV HRIR de 14 canais compatível com o layout usado pelo exemplo
  oficial `sink-virtual-surround-7.1-hesuvi.conf` do PipeWire;
- localização padrão:
  `$XDG_CONFIG_HOME/jambalinux-soniccore/spatial/hrtf/hrir.wav`;
- manifesto ao lado do arquivo, por exemplo `manifest.json`, contendo schema,
  formato, nome descritivo neutro, SHA-256, licença e URL de origem;
- aceitar somente arquivo regular e caminho canônico contido no diretório
  HRIR; rejeitar links ou caminhos que escapem desse diretório;
- validar a estrutura WAV e exatamente 14 canais antes de criar o grafo;
- aceitar apenas taxas/amostras que o convolver do PipeWire instalado consiga
  ler; erros devem ser apresentados antes de ativar o recurso.

O repositório e o instalador não devem baixar nem incluir automaticamente uma
HRIR. A inclusão futura de um dataset só pode ocorrer após revisão explícita da
licença e da proveniência. Testes automatizados podem gerar uma WAV sintética
com impulsos Dirac; isso valida canais e roteamento, não qualidade binaural.

## Fases

### 1. Contrato e validação

1. Trocar o teste atual de “diretório não vazio” por validação real de
   `manifest.json` e `hrir.wav`.
2. Separar funções puras de validação para testes com diretórios temporários.
3. Fazer o preflight informar caminho, formato, hash, validade e erro preciso.
4. Manter `enable` recusando ativação quando o preflight não estiver pronto.

### 2. Renderizador do grafo

1. Criar um módulo separado, sugerido `src/spatial_pipewire.rs`.
2. Renderizar configuração própria baseada no exemplo 7.1 oficial instalado,
   sem editar arquivos globais do PipeWire.
3. Usar oito entradas na ordem `FL FR FC LFE RL RR SL SR`.
4. Usar as 14 respostas do WAV segundo o mapeamento do exemplo oficial,
   tratando LFE como FC na primeira versão.
5. Produzir apenas `FL FR` e usar `target.object` apontando para
   `jambalinux-soniccore-game-equalizer`.
6. Escapar todos os caminhos inseridos na configuração.
7. Usar nomes neutros e estáveis:
   `jambalinux-soniccore-game-spatial` e
   `jambalinux-soniccore-game-spatial-output`.

### 3. Serviço persistente e ciclo de vida

1. Adicionar `jambalinux-soniccore-spatial.service`, iniciado na sessão do
   usuário e ordenado depois do serviço do equalizador.
2. Implementar `soniccore spatial run` como supervisor persistente: sem perfil
   habilitado, não publica sink; habilitado e válido, mantém um único processo
   `pipewire -c <config>` e recupera após reconexão.
3. Nunca reiniciar PipeWire, WirePlumber ou o serviço do equalizador ao alternar
   o modo.
4. Ao desativar, mover de forma controlada streams ainda conectados à entrada
   espacial para a entrada do equalizador antes de remover o grafo.
5. Persistir somente o mínimo necessário para recuperação e restauração.
6. Falhar de forma fechada se o equalizador de destino estiver ausente ou se
   houver mais de um alvo compatível.

### 4. Estado e saúde

Estender `soniccore spatial status --format json` com, no mínimo:

- `configured`, `enabled`, `active` e `service_active`;
- `dataset_valid`, caminho e SHA-256 esperado/observado;
- IDs/serials dos nós de entrada e saída;
- `target_equalizer_connected`;
- `default_safe` — nenhum sink virtual virou padrão por ação do projeto;
- `chat_isolated` e `capture_isolated`;
- `routing_healthy` e erro acionável;
- formato de entrada 7.1 e saída estéreo observados;
- latência reportada pelo grafo quando disponível.

O status só pode dizer `active: true` quando o grafo e os links forem
observáveis e saudáveis.

### 5. CLI e widget

1. Preservar `spatial preflight`, `status`, `enable`, `disable` e `mode`.
2. Fazer `enable` registrar intenção; o serviço persistente materializa o
   grafo e o status confirma o resultado.
3. Depois do backend validado, transformar a seção **Espacial** do widget em
   controle de ativação/desativação.
4. Mostrar claramente dataset ausente, grafo preparando, ativo, erro e a
   necessidade de selecionar a saída espacial no jogo.
5. O widget nunca deve baixar arquivos, alterar saída padrão ou executar
   comandos livres fornecidos pelo usuário.

### 6. Testes automatizados

- manifesto ausente, inválido, hash divergente e caminho inseguro;
- WAV com canal/estrutura incorretos;
- renderização determinística e escaping de caminho;
- oito entradas, dois outputs e mapeamento correto dos 14 convolvers;
- saída apontando somente ao equalizador JamBaLinux;
- nomes proibidos ausentes de modos, IDs e descrições do produto;
- status não ativo quando processo, nós, links ou dataset faltarem;
- Chat e qualquer captura jamais aceitos como destino;
- desativação sempre possível, mesmo com dataset removido;
- rollback após falha de criação ou persistência;
- testes atuais de equalizador, HID e espacial permanecendo verdes.

### 7. Validação na máquina real

Realizar somente depois dos testes e com autorização do mantenedor:

1. Registrar baseline de saída padrão, nós, links, serviço do EQ e perfil.
2. Usar primeiro uma HRIR sintética de 14 canais com impulsos conhecidos.
3. Reproduzir identificação de canais 7.1 e confirmar que cada canal aparece
   nos lados/atrasos esperados por correlação, não apenas “parece funcionar”.
4. Repetir com uma HRIR aberta cuja licença e hash estejam documentados.
5. Confirmar cadeia `aplicação -> spatial -> EQ -> Game`.
6. Confirmar que alterar o EQ ainda afeta a saída final.
7. Confirmar que Chat, microfone e saída padrão permanecem inalterados.
8. Medir latência adicional, CPU, xruns e continuidade por pelo menos dez
   minutos; registrar os números sem alegar paridade com software proprietário.
9. Desativar durante reprodução e confirmar fallback estéreo controlado.
10. Testar desconexão/reconexão do dongle e restauração do grafo.

## Critérios de aceite do MVP

- Ativação é opt-in e recusada sem dataset válido.
- Uma aplicação que abre o sink espacial em 7.1 chega ao headset em estéreo.
- Os oito canais contribuem para a saída conforme o mapeamento documentado.
- O EQ continua funcional depois da binauralização.
- Nenhuma alteração USB/HID, saída padrão, Chat ou microfone ocorre.
- Desativar remove o grafo e preserva/reencaminha streams de modo seguro.
- Reconnect recupera serviço e links sem criar nós duplicados.
- Erros são observáveis e acionáveis no CLI/widget.
- Testes, documentação, instalador e desinstalador são atualizados.

## Fora do escopo inicial

- head tracking;
- tamanhos de sala ou diâmetro da cabeça;
- clonagem de DTS/Quantum Spatial/Dolby;
- download ou redistribuição automática de HRIR;
- seleção automática do sink como padrão;
- roteamento automático de todos os aplicativos;
- processamento de Chat ou microfone;
- perfis 5.1, 7.1.4, Atmos ou SOFA antes de o MVP 7.1 estar medido.

## Trabalho engatilhado após as fases de áudio espacial

Não implementar os itens abaixo durante as fases 1–7. Eles formam uma rodada
separada de acabamento da interface e evolução do equalizador, iniciada apenas
depois da conclusão e validação do áudio espacial:

1. Remover da interface as duas mensagens informativas redundantes:
   - `Automático · ativo na rota Game: <nome-da-rota>`;
   - `A rota Game é processada automaticamente; Chat, microfone e a saída padrão permanecem fora da cadeia.`
2. Permitir que uma configuração manual do equalizador seja salva como perfil
   personalizado com nome escolhido pelo usuário.
3. Permitir renomear/editar e excluir perfis personalizados, preservando os
   perfis predefinidos do aplicativo.
4. Antes de implementar, definir persistência, validação de nomes, tratamento
   de duplicatas, confirmação de exclusão e migração compatível do formato já
   salvo.

## Proibições de segurança

- Não alterar VID/PID, matching USB/HID, allowlists ou protocolo do headset.
- Não adicionar raw reports, reset, firmware ou comandos adivinhados.
- Não escrever em `/etc` ou reiniciar a pilha de áudio inteira.
- Não apagar configurações do usuário nem sobrescrever arquivos sem backup e
  propriedade clara.
- Não usar ativos proprietários ou nomes de fornecedor como nome de recurso.
- Não declarar sucesso apenas porque o sink apareceu; medir canais, links,
  isolamento, fallback e efeito audível.
