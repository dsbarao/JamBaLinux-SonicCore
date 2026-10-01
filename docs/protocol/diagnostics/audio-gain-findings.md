# Portão de decisão — ganho de áudio

**Estado:** decisão de diagnóstico passivo parcialmente substituída pela constatação causal EVT-046 (2026-09-30).  
**Decisão:** **autorizada a correção de perfil ACP no estágio de mapeamento de mixer/hardware (Game/Chat); também autorizada a proteção digital de headroom (preamp automático não positivo no EQ e orçamento de atenuação no Spatial), por ser derivável e verificável matematicamente sem medição no host; para os demais estágios, permanece a decisão AG05 de evidência insuficiente — não implementar alterações não relacionadas de roteamento, HID, coeficientes, HRIR ou resampling.**  
**Escopo analisado:** mapeamento ALSA Card Profile (ACP) de `0ecb:2069` (`hw:%f,0,0` / `hw:%f,1,0`) e controles de mixer `PCM,0` / `PCM,1`. Caminho downstream: `aplicação -> Spatial -> EQ -> Quantum Game`; Quantum Chat como endpoint comparador.  
**Limite de segurança:** este registro não altera HID, VID/PID, allowlists, seleção de dispositivo, bytes de protocolo ou arquivos do sistema sob `/usr`. A correção autorizada limita-se à sobreposição de perfil ACP reversível em nível de usuário (`tools/install-user.sh` / `uninstall-user.sh`). A proteção digital de headroom do EQ e do Spatial só pode atenuar (nunca aplicar ganho positivo) e não altera bandas, coeficientes, HRIR ou rotas. Bloqueios a mudanças não relacionadas de roteamento, HID, coeficientes, HRIR e resampling permanecem ativos.

## Proveniência revisada

| Item | Evidência revisada | Limite da evidência |
|---|---|---|
| Software | `jambalinux-soniccore` 0.2.0, revisão `d2d8c363def9a4a1dc09f34936d822c7e96b2422` (2026-09-26) | Identifica o código/documentação revisados; não identifica uma sessão PipeWire nem o hardware atual. |
| Mapeamento ACP | EVT-046 (2026-09-30): regra udev upstream associa `0ecb:2069` ao profile-set `usb-gaming-headset-gamefirst.conf`. Mapeamento `stereo-game-output` (`hw:%f,0,0`) usa `paths-output = usb-gaming-headset-output-stereo` com `[Element PCM,1]`. | Demonstra e confirma exclusivamente a inversão/desconexão do controle ALSA de hardware entre Game e Chat; não atribui efeito a estágios de processamento digital downstream. |
| Validação manual | Ajuste de mixer de hardware `PCM,0` a 100% (0 dB) e persistência via `alsactl store` (EVT-046). | Restaura volume/fidelidade do hardware Game (anteriormente retido em -23 dB); valida a causa no estágio ACP/mixer. Não mede SPL acústico nem HRIR. |
| Contrato e observador | [audio-gain.md](audio-gain.md), seções ag01–ag03 | Definem campos e análise offline; nenhum log passivo de um host foi anexado a esta decisão. |
| Rota documentada | `aplicação -> Spatial (7.1→estéreo) -> EQ -> Quantum Game`; Chat isolado, conforme [audio-gain.md](audio-gain.md) | É caminho pretendido, não prova links, seriais, volumes ou formatos negociados em uma sessão. |
| Rota exercitada offline | `game-bypass.json` exercita o modelo Game em bypass, e `chat-binaural.json` o modelo Chat em binaural, ambos processados pelo ledger | Não há links de uma rota real: os fixtures não carregam destino, nó ou `object.serial`; não tratá-los como teste do headset. |
| Protocolo A/B | [audio-gain-ab.md](audio-gain-ab.md) e `tests/fixtures/audio_gain/ab-protocol-manifest-template.json` | Pré-registro não executado: não há pares AB/BA, três repetições, captura pré/pós ou restauração a revisar. |
| Tabelas offline | `game-bypass.json`, `chat-binaural.json`, `format-rate-divergent.json`, `positive-eq.json` e `missing-fields.json`, processados por `tools/analyze_audio_gain.py` | São fixtures sintéticas e reprodutíveis, não logs do usuário, medições no headset ou capturas de áudio. |
| Evidência de roda | EVT-009 no [evidence-ledger.md](../evidence-ledger.md) | Confirma posições do dial e ALSA inalterado naquela observação; não mede ganho digital, endpoint, SPL ou percepção. |

Não foi executado observador contra PipeWire, coleta de áudio, mudança de rota, volume, Spatial, EQ ou roda física nesta fase. Portanto não há `pw-dump`, `pactl`, serial de nó, log de versão de PipeWire, captura PCM ou tabela A/B do host para inferir causalidade.

## Resultados offline revisados

| Fixture / condição modelada | Observável no relatório | Decisão limitada ao estágio |
|---|---|---|
| `game-bypass.json` — bypass nominal, Game/Chat F32LE 48 kHz e volumes 1,0 | O limite derivado da matriz seca é 3,121 / +9,886 dBFS para entradas correlacionadas; não há pico medido. | **Downmix seco:** risco teórico de soma; não há evidência para mudar coeficientes. |
| `chat-binaural.json` — binaural nominal | Game F32LE/48 kHz e Chat S16LE/44,1 kHz divergem; ganho de convolver/HRIR ausente. | **Conversão/resampling:** confundidor documentado, não causa medida. |
| `format-rate-divergent.json` — wet/dry 0,5/0,5 | A transição não identifica modo estável e formatos/taxas divergem. | **Rampa Props:** estado de transição inválido para atribuição de ganho. |
| `positive-eq.json` — EQ +6 dB e +12 dB | Pico sintético pós-EQ de +0,2 dBFS, sem preamp automático. | **EQ:** clipping no fixture bloqueia compensação; não demonstra clipping no sistema do usuário. |
| `missing-fields.json` — campos ausentes | Wet/dry, formatos, volumes, EQ e picos permanecem indeterminados. | **Insuficiência de evidência:** os campos mínimos são necessários antes de qualquer conclusão. |

As tabelas não misturam domínios: RMS/pico dBFS são medições digitais; volume de stream/sink é controle exposto; SPL e espacialidade são percepção/medição acústica separadas. Nenhum deles é substituto do outro.

## Matriz causal e decisão

Cada linha abaixo trata de exatamente um estágio causal. “Pendente” não é uma conclusão de defeito: significa que os dados revisados não discriminam a hipótese.

| Estágio causal | Evidência observável revisada | Conclusão | Medição que poderia justificá-lo | Correção que só poderia ser considerada após a métrica |
|---|---|---|---|---|
| Perfil ACP / Controles PCM (Game/Chat) | EVT-046: upstream associa `0ecb:2069` a `usb-gaming-headset-gamefirst.conf`, mapeando `stereo-game-output` para `[Element PCM,1]` (Chat). `PCM,0` (Game) nunca era controlado e permaneceu em -23 dB. Validação com `PCM,0` a 100% (0 dB) restaurou volume/fidelidade nominais. | **Causa confirmada (estágio ACP/mixer):** substitui parcialmente o bloqueio da decisão AG05 exclusivamente para correção do perfil ACP de Game/Chat. | Validação em host via ajuste de controle ALSA `PCM,0` a 0 dB e persistência com `alsactl store` (concluída em EVT-046). | Entrega de overlay ACP de usuário reversível (`tools/install-user.sh` / `uninstall-user.sh`) mapeando Game para `PCM,0` e Chat para `PCM,1`. Nenhuma alteração sob `/usr`, sem alteração HID, VID/PID ou allowlists. |
| Downmix 7.1→estéreo | Matriz seca documentada e limite teórico +9,886 dBFS; sem correlação por canal nem pico pré/pós. | **Pendente:** não há canal ausente, soma divergente ou clipping medido. | Estímulos isolados nos oito canais, correlação/atraso/crosstalk L/R e soma prevista versus observada, com pico/true-peak ≤0 dBFS. | Alterar coeficiente somente se a matriz observada divergir da matriz especificada, mantendo headroom demonstrado. |
| HRIR / convolver | Topologia de 16 convolvers e 14 canais HRIR; hash, RMS/pico e normalização ausentes. | **Pendente:** ganho/normalização HRIR indeterminado. | Hash/manifesto, taxa/canais/duração e RMS/pico de cada HRIR e dos 16 caminhos, mais A/B Spatial repetido. | Normalizar ou trocar HRIR somente se a perda/excesso for repetível no caminho wet e atribuível a esses níveis. |
| Gate/rampas `Props` | Estados nominais wet/dry e fixture em transição; sem valores lidos por instante de captura. | **Pendente:** não há rampa aplicada mensurada. | Quatro `Props` observados, intervalo real e captura fora da transição em AB/BA. | Alterar rampa somente se valores/tempo aplicados divergirem e a divergência for correlacionada à medição digital. |
| EQ / headroom | Faixa permitida −12 a +12 dB, ausência de preamp; fixture +0,2 dBFS. | **Pendente no host:** o fixture demonstra o bloqueio, não clipping real. | RMS/pico/true-peak pré/pós por faixa, bandas solicitadas/lidas e repetição Flat versus perfil. | Autorizado como proteção digital: preamp automático não positivo derivado da resposta combinada dos biquads, que impede ganho calculado acima de 0 dBFS sem depender de medição no host. Limiter ou ajustes além disso continuam exigindo medição; não elevar volume acima de 100%. |
| Endpoint Game/Chat (roteamento/links) | EVT-009, EVT-046 e fixtures; rota de processamento downstream. | **Pendente quanto a mudanças de roteamento:** a causa raiz foi isolada no perfil ACP/mixer de hardware, não no roteamento PipeWire downstream. | Manter rota downstream inalterada; verificar restauração de ganho pelo mapeamento correto dos sinks aos PCMs. | Manter isolamento estrito de Chat e não alterar a rota downstream `aplicação -> Spatial -> EQ -> Game`. |
| Conversão / resampling | Divergência F32LE/48 kHz versus S16LE/44,1 kHz em fixture. | **Pendente:** formato divergente é confundidor, não degradação comprovada. | Formato/taxa/mapa efetivamente negociados por link e captura, com par compatível ou divergência isolada. | Alterar estratégia de resampling somente se uma conversão específica for observada e explicar diferença digital repetível. |
| Volume de stream/sink | Valores de controle aparecem apenas em fixtures; nenhum nível de amostra ou SPL de host. | **Pendente:** controle não prova ganho digital nem percepção. | Volumes linear/dB e mute, junto a captura alinhada pré/pós; SPL só em medição acústica separada. | Compensação de ganho somente se o estágio causal já isolado tiver margem digital e métrica de ganho repetível. |
| Roda Game/Chat | EVT-009 observa posições HID e ALSA inalterado; EVT-046 restringe a causa confirmada ao estágio ACP/PCM e não atribui efeito ao dial HID. | **Pendente:** o dial físico não foi medido para ganho, endpoint ou loudness; a evidência confirma apenas o estágio ACP/Game/Chat e não atribui efeito ao dial HID. | Par observacional aprovado, com posição apenas anotada e endpoints/volumes/capturas passivos; sem ler ou escrever HID. | **Nenhuma alteração HID.** Qualquer investigação HID exige validação explícita em hardware e revisão própria. |

## Critério de liberação de uma única fase corretiva

A decisão AG05 original foi substituída **exclusivamente** para a correção do estágio de perfil ACP (Game/Chat), liberando a fase de entrega do overlay ACP de usuário reversível.

Também fica autorizada a proteção digital de headroom: preamp automático não positivo no EQ e orçamento conservador de atenuação no Spatial (HRIR e downmix 7.1), com falha fechada quando o limite não puder ser calculado. Ela só atenua e é verificável por testes determinísticos, sem depender de medição no host.

Para todos os outros estágios (alterações de coeficientes do downmix, normalização de HRIR, gate/rampas, roteamento downstream, resampling e HID):
1. A evidência de EVT-046 confirma estritamente o desacoplamento de hardware ACP/PCM, sem atribuir efeitos a SPL, HRIR, dial HID ou resampling.
2. Os bloqueios da decisão AG05 permanecem rigorosamente em vigor para mudanças de roteamento downstream, manipulações HID, normalizações de HRIR ou alterações não medidas de DSP.
3. Não são permitidas alterações em arquivos do sistema (`/usr`), allowlists, VID/PID ou protocolos HID.
4. Qualquer fase subsequente para os demais estágios continua dependendo de evidência pré-registrada e delimitada que satisfaça os critérios objetivos do protocolo.
