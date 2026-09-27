# Portão de decisão — ganho de áudio

**Estado:** decisão de diagnóstico passivo; nenhuma fase corretiva está autorizada.  
**Decisão:** **evidência insuficiente — não implementar alteração de áudio.**  
**Escopo analisado:** `aplicação -> Spatial -> EQ -> Quantum Game`; Quantum Chat é somente o endpoint comparador.  
**Limite de segurança:** este registro não altera HID, VID/PID, allowlists, seleção de dispositivo ou bytes de protocolo. O relato da roda Game/Chat não autoriza hipótese nem alteração HID sem validação física explícita e revisão separada.

## Proveniência revisada

| Item | Evidência revisada | Limite da evidência |
|---|---|---|
| Software | `jambalinux-soniccore` 0.2.0, revisão `d2d8c363def9a4a1dc09f34936d822c7e96b2422` (2026-09-26) | Identifica o código/documentação revisados; não identifica uma sessão PipeWire nem o hardware atual. |
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
| Downmix 7.1→estéreo | Matriz seca documentada e limite teórico +9,886 dBFS; sem correlação por canal nem pico pré/pós. | **Pendente:** não há canal ausente, soma divergente ou clipping medido. | Estímulos isolados nos oito canais, correlação/atraso/crosstalk L/R e soma prevista versus observada, com pico/true-peak ≤0 dBFS. | Alterar coeficiente somente se a matriz observada divergir da matriz especificada, mantendo headroom demonstrado. |
| HRIR / convolver | Topologia de 16 convolvers e 14 canais HRIR; hash, RMS/pico e normalização ausentes. | **Pendente:** ganho/normalização HRIR indeterminado. | Hash/manifesto, taxa/canais/duração e RMS/pico de cada HRIR e dos 16 caminhos, mais A/B Spatial repetido. | Normalizar ou trocar HRIR somente se a perda/excesso for repetível no caminho wet e atribuível a esses níveis. |
| Gate/rampas `Props` | Estados nominais wet/dry e fixture em transição; sem valores lidos por instante de captura. | **Pendente:** não há rampa aplicada mensurada. | Quatro `Props` observados, intervalo real e captura fora da transição em AB/BA. | Alterar rampa somente se valores/tempo aplicados divergirem e a divergência for correlacionada à medição digital. |
| EQ / headroom | Faixa permitida −12 a +12 dB, ausência de preamp; fixture +0,2 dBFS. | **Pendente no host:** o fixture demonstra o bloqueio, não clipping real. | RMS/pico/true-peak pré/pós por faixa, bandas solicitadas/lidas e repetição Flat versus perfil. | Preamp ou limiter somente se clipping digital real for repetido e a margem necessária for medida; não elevar volume acima de 100%. |
| Endpoint Game/Chat | EVT-009 e fixtures; nenhum par de endpoints real. | **Pendente:** diferença de endpoint, ganho elétrico e SPL não foram medidos. | Mesmo sinal/nível, rota/seriais, volumes, formatos e RMS/pico de Game versus Chat; SPL separado se disponível. | Mudar roteamento somente se o par isolar endpoint como única variável e a rota original puder ser restaurada/verificada. |
| Conversão / resampling | Divergência F32LE/48 kHz versus S16LE/44,1 kHz em fixture. | **Pendente:** formato divergente é confundidor, não degradação comprovada. | Formato/taxa/mapa efetivamente negociados por link e captura, com par compatível ou divergência isolada. | Alterar estratégia de resampling somente se uma conversão específica for observada e explicar diferença digital repetível. |
| Volume de stream/sink | Valores de controle aparecem apenas em fixtures; nenhum nível de amostra ou SPL de host. | **Pendente:** controle não prova ganho digital nem percepção. | Volumes linear/dB e mute, junto a captura alinhada pré/pós; SPL só em medição acústica separada. | Compensação de ganho somente se o estágio causal já isolado tiver margem digital e métrica de ganho repetível. |
| Roda Game/Chat | EVT-009 observa posições HID e ALSA inalterado. | **Pendente:** o dial não foi ligado a ganho, endpoint ou loudness. | Par observacional aprovado, com posição apenas anotada e endpoints/volumes/capturas passivos; sem ler ou escrever HID. | **Nenhuma alteração HID.** Qualquer investigação HID exige validação explícita em hardware e revisão própria. |

## Critério de liberação de uma única fase corretiva

Uma próxima fase só pode escolher **um** estágio da matriz se entregar dados sanitizados que satisfaçam simultaneamente o protocolo A/B: mesma fonte e nível, rota/serial e volumes registrados, formatos negociados e capturas comparáveis, AB/BA balanceado com ao menos três repetições, alinhamento, tolerância de 0,2 dB para tom estável, e ausência de clipping/true-peak acima de 0 dBFS. Ela deve citar a linha causal correspondente e manter as demais como pendentes; se os dados não discriminarem, a decisão permanece “não implementar”.

Em particular, nenhuma métrica atual justifica preamp, limiter, coeficientes, normalização de HRIR, resampling, roteamento ou compensação de volume. A ação correta nesta decisão é preservar o estado e aguardar aprovação humana para a sessão A/B ativa descrita no protocolo.
