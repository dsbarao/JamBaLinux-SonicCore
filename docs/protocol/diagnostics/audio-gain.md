# Contrato de evidência — ganho de áudio

**Estado:** plano de diagnóstico passivo; não é uma medição e não autoriza
mudança de áudio, PipeWire, HID ou hardware.  
**Escopo:** caminho autorizado `aplicação -> Spatial -> EQ -> Quantum Game`.

Este contrato separa o que o grafo e a documentação já permitem observar do
que ainda precisa de medição. Ele existe antes de qualquer hipótese de correção
para o relato de espacial baixo, diferença histórica do dial Game/Chat e
degradação/clipping ao elevar EQ ou o volume acima de 100%.

## Limites inegociáveis

Nesta fase são proibidos `pactl move-sink-input`, `pactl set-sink-volume`,
`pactl set-sink-input-volume`, `pactl set-default-sink`, `pw-cli set-param`,
`pw-link`, `wpctl set-volume` e qualquer outro comando que escreva no grafo.
Também são proibidos mudança de volume, movimentação de stream, troca do modo
espacial ou EQ, reinício de serviço, criação/remoção de links e qualquer escrita HID.
Consultas somente-leitura e análise offline de capturas já obtidas são a
única atividade permitida.

Nenhuma observação deste contrato autoriza alterar VID/PID, allowlists, seleção
de dispositivo, bytes de relatórios HID ou comportamento de protocolo. O dial
Game/Chat só pode ser correlacionado com evidência passiva; uma eventual mudança
HID exigiria validação explícita em hardware e revisão separada.

## Inventário de estágios e evidência exigida

| Estágio | Fato observável a registrar | Unidades/campos mínimos | Não concluir sem |
|---|---|---|---|
| Stream da aplicação | Identidade, destino e volume efetivo do stream | `pactl list sink-inputs`: índice, `Sink`, `Volume` (linear e dB quando exposto), `Corked`, `application.name`, `media.name`, `format` | Captura PCM pré- e pós-estágio; `Volume` não é nível de amostra |
| Entrada Spatial | Nó/serial e links de entrada 7.1 | `pw-dump`: `id`, `object.serial`, `node.name`, `media.class`, `audio.position`, `audio.rate`, `audio.channels`, `format.dsp`/formato negociado e links | Prova de que um cliente estéreo foi realmente promovido para 7.1 |
| Mixers seco/wet | Valores atuais dos quatro controles do nó de captura Spatial | `Params.Props`: `wetDryL:Gain 1`, `wetDryR:Gain 1`, `wetDryL:Gain 2`, `wetDryR:Gain 2`; linear e dB calculado quando positivo | Que a rampa ocorreu no instante da captura ou que os ganhos são normalizados perceptualmente |
| Convolvers/HRIR | Dataset, 16 caminhos e níveis por canal na captura | hash/manifesto, taxa, canais, duração/resposta ao impulso; RMS/pico L/R por entrada | Normalização, ganho unitário, fase ou qualidade perceptiva da HRIR |
| Downmix seco | Coeficientes renderizados e correlação por canal | matriz: L=`FL + .707 FC + .707 SL + .707 RL`; R=`FR + .707 FC + .707 SR + .707 RR`; LFE omitido | Nível agregado sob conteúdo correlacionado ou comportamento de upmix do cliente estéreo |
| EQ | Nó/serial, dez ganhos e nível antes/depois do filtro | `Props`, bandas em dB, RMS/pico dBFS, formato/taxa | Ausência de clipping apenas porque os ganhos configurados parecem moderados |
| Saída EQ / endpoint | Link para o Game físico, formato negociado e volume do sink | `pw-dump` e `pactl list sinks`: nó, serial, `alsa.components`, `alsa.device`, `api.alsa.pcm.stream`, `Volume`, `Base Volume`, formato/taxa/canais | Relação entre controle do sink e SPL no headset |
| Quantum Game e Chat | Identificadores distintos, links, volumes e formatos de ambos | nome/índice/serial, `alsa.components`, PCM, volume linear/dB, formato/taxa/canais | Que o dial muda ganho de software, endpoint ou loudness; o relatório HID apenas identifica posição |
| Volume percebido | Resultado humano separado da telemetria | protocolo A/B posterior, condição, posição física, escala subjetiva e, se disponível, SPL calibrado | Inferir percepção de RMS, pico ou volume de controle |

Para todos os nós, guardar a amostra bruta sanitizada de `pw-dump` e a relação
`id`/`object.serial`: IDs podem mudar numa recriação; serial não deve ser
comparado como se fosse estável entre sessões. Para formatos, registrar tanto
`audio.format` quanto taxa, canais e `audio.position` efetivamente negociados
em cada link disponível. Esses são os **formatos efetivamente negociados**;
uma propriedade declarada pelo nó não prova, sozinha, o formato do fluxo.

## Fatos já documentados

- O caminho pretendido é `aplicação -> Spatial -> EQ -> Quantum Game`; Chat e
  captura ficam isolados. O Spatial é 7.1 de entrada e estéreo de saída.
- O downmix seco usa os coeficientes acima e omite LFE. Para uma fonte que usa
  somente FL/FR, ele é nominalmente identidade estéreo.
- O Spatial possui 16 convolvers a partir de 14 canais HRIR; as duas respostas
  de LFE reutilizam as de FC. Isto descreve a topologia, não o ganho real.
- O gate persistido seleciona nominalmente wet=1/dry=0 para binaural e
  wet=0/dry=1 para bypass. A implementação atual usa quatro atualizações
  `Props` com três intervalos de 22 ms para a rampa do Spatial; a duração real
  e cada valor aplicado só são confirmáveis por observação do nó.
- O EQ aceita bandas de -12 a +12 dB e não há preamp automático. Portanto,
  somas espectrais e ganhos positivos podem exceder 0 dBFS e clipar.
- EVT-009 confirma que o dial físico emite posições Game/Chat e que ALSA não
  mudou nessa observação. Isso não mede ganho, endpoint, volume percebido nem
  explica o comportamento histórico relatado.
- EVT-046 confirma a causa raiz no profile-set ACP upstream
  (`usb-gaming-headset-gamefirst.conf` associado a `0ecb:2069` via
  `/usr/lib/udev/rules.d/90-pipewire-alsa.rules`): o mapeamento
  `stereo-game-output` (`hw:%f,0,0`) usa `paths-output = usb-gaming-headset-output-stereo`
  que controla apenas `[Element PCM,1]` (volume de hardware do Chat). O controle de
  hardware `PCM,0` (Game) nunca era controlado pelo sink Game do PipeWire, tendo ficado
  atenuado (-23 dB). O ajuste manual de `PCM,0` para 100% (0 dB) e sua persistência
  restauraram o volume e a fidelidade nominais. Essa evidência confirma estritamente
  o estágio ACP/mixer de hardware e não atribui efeito ao dial HID, SPL, HRIR,
  resampling ou outros estágios.

## Hipóteses explicitamente não confirmadas

- A HRIR pode estar normalizada, atenuada, amplificada ou ter energia desigual
  entre caminhos; nenhuma dessas possibilidades está confirmada.
- O comportamento de `channelmix` para cliente estéreo conectado ao sink 7.1
  (upmix estéreo->7.1, distribuição de centro/LFE/surround e ganho) não está
  confirmado em hardware.
- Não está confirmado que o dial Game/Chat altere ganho no host, no endpoint,
  no headset, ou apenas a experiência percebida. Não inferir isso do byte HID.
- Conversão de formato, remapeamento de canal, resampling e seus ganhos não
  estão confirmados; podem ocorrer em qualquer fronteira negociada.
- Nem `Props` do gate, nem ganhos de EQ, nem volume de stream/sink demonstram
  por si só ausência de clipping, headroom suficiente ou equivalência de SPL.

## Contrato de medição offline

Capturas futuras devem ser analisadas offline, por janela, usando a mesma
referência digital em cada condição. Para cada canal disponível, registrar:

- RMS em dBFS: `20 log10(rms / full_scale)`; pico em dBFS:
  `20 log10(max(abs(amostra)) / full_scale)`. Reportar `-inf` para silêncio e
  a definição de `full_scale` (por exemplo, 1.0 float ou 32768 S16).
- Pico true-peak, se a ferramenta de análise o suportar, identificado
  separadamente do pico de amostra. Qualquer pico de amostra ou true-peak
  maior que 0 dBFS reprova a condição como sem headroom demonstrado.
- Correlação normalizada por entrada 7.1 e saída L/R, atraso do máximo de
  correlação e crosstalk em dB. O objetivo é atribuir canais, não declarar
  qualidade espacial por uma única correlação.
- Diferença de RMS e de pico entre pré/post de cada estágio, em dB, e soma
  prevista versus observada no downmix. Para sinais correlacionados, a soma dos
  coeficientes pode aumentar pico/RMS; não tratar 0.707 como garantia de não
  clipping.
- Formato do arquivo capturado (codec/PCM, bits ou float, taxa, canais, ordem)
  e formato negociado no grafo. Recusar comparação se esses dados, o alinhamento
  temporal ou a fonte não forem equivalentes.

### Limites de erro e rejeição

As medições devem declarar resolução, janela e ferramenta. Aceitar somente
comparações com janelas alinhadas e diferença de taxa de amostragem tratada
explicitamente. Para tom estável e mesma cadeia, usar tolerância de até 0,2 dB
para RMS/pico e até uma amostra para o atraso esperado; fora disso, repetir e
marcar inconclusivo, não atribuir causa. Para sinais musicais, não aplicar esses
limites sem alinhamento e repetição: reportar intervalo/variância.

Rejeitar uma conclusão de ganho se faltar: identificação de nó/serial, volumes
lineares e dB disponíveis, formatos negociados, captura pré/pós comparável,
ou separação entre medição elétrica/digital e percepção. Rejeitar uma conclusão
de downmix se os oito canais não tiverem correlação por canal. Rejeitar uma
conclusão sobre HRIR se o hash/manifesto, normalização e pico/RMS dos caminhos
não forem conhecidos.

## Sequência passiva para as próximas fases

## Observador de grafo (ag02)

`tools/diagnose-audio-gain.sh` materializa a primeira etapa sem mudar o
grafo. Ele consulta somente `pactl`, `pw-dump`, `pw-metadata` quando presente,
os status JSON do SonicCore e metadados do arquivo HRIR apontado pelo status.
Não cria captura de áudio, não cria arquivo temporário de áudio e não escreve
em PipeWire, no estado SonicCore ou em HID.

```bash
tools/diagnose-audio-gain.sh --duration 30 --interval 1 --output /caminho/novo.log
```

`pactl`, `pw-dump` e `soniccore` são obrigatórios; uma ausência falha com o
nome da ferramenta. `pw-metadata` e `sha256sum` são opcionais: seu bloco é
marcado como `unavailable`, sem preencher valores supostos. Cada amostra tem
`monotonic_us` obtido de `/proc/uptime`, além de hora de parede para correlação
humana, e conserva os resultados brutos delimitados de `pactl list sinks`,
`pactl list sink-inputs`, `pw-dump`, metadados e três status JSON. Assim ficam
observáveis volumes/mute, Game/Chat quando anunciados, nós/links, `Props`,
wet/dry, formatos, taxas, mapas de canal e conversões expostas — mas não se
infere que uma propriedade anunciada seja um formato negociado ou um ganho
audível.

O caminho padrão usa um arquivo novo sob `$XDG_RUNTIME_DIR`; um destino já
existente é recusado. O log pode conter `application.name` e `media.name`, que
podem ser sensíveis, e a ferramenta avisa antes da coleta e no próprio arquivo.
O observador não rotula bytes HID, não altera VID/PID ou allowlists, e não
relaciona a posição Game/Chat a ganho sem o A/B posterior.

1. Coletar instantâneos somente-leitura simultâneos de `pw-dump`, `pactl list
   sinks`, `pactl list sink-inputs`, `pactl get-default-sink` e os status JSON
   já existentes. Registrar comandos, hora monotônica, saída, nó/serial e
   ausência de erro, sem corrigir o que for observado.
2. Montar offline um orçamento de ganho que mantenha separadas as variáveis:
   volume do stream, controles wet/dry, energia HRIR/convolvers, soma do
   downmix, bandas EQ, volume do sink, endpoint e conversão/resampling.
3. Só após aprovação humana para uma fase ativa, executar A/B objetivo com
   fonte idêntica, uma única variável por vez, captura digital pré/post e
   condição de retorno. A/B de Game/Chat deve medir os dois endpoints e a
   posição HID somente como rótulo, sem escrever HID.

Este documento não recomenda elevar volumes, usar valores acima de 100%,
compensar ganho, alterar coeficientes, adicionar preamp, trocar HRIR ou mudar
roteamento. Essas seriam implementações posteriores, condicionadas à evidência
e às autorizações adequadas.

## Ledger offline (ag03)

`tools/analyze_audio_gain.py` não consulta uma sessão PipeWire, não executa
SonicCore e não aceita argumentos que descrevam um dispositivo. Ele lê somente
um arquivo JSON fornecido pelo operador e escreve um relatório JSON em stdout:

```bash
python3 tools/analyze_audio_gain.py captura-sanitizada.json --pretty
```

O arquivo deve declarar `"schema": "jambalinux-audio-gain-ledger/v1"`.
Ele é a transcrição explícita e sanitizada de campos presentes no instantâneo
passivo: `streams`, `spatial.wet_dry_gains`, `spatial.formats`,
`eq.requested_bands_db`, `eq.applied_bands_db`, `sinks.game`, `sinks.chat` e,
quando houver captura digital comparável, `measured_peaks_dbfs`. Formatos usam
`format`, `rate`, `channels` e opcionalmente `position`; ganhos de stream/sink
continuam observações, não níveis de amostra. As fixtures em
`tests/fixtures/audio_gain/` são exemplos completos e reprodutíveis desse
formato, incluindo Game/Chat, bypass/binaural, EQ positivo, lacunas e taxas
divergentes.

O relatório mantém `observed` separado de `limits_derived`. O único limite de
soma que ele calcula é a matriz seca documentada: máximo correlacionado por
lado `1 + 3*0.707 = 3.121` (cerca de `+9.887 dBFS` com entrada full-scale).
Isso é risco teórico, não medição de pico. O headroom contratual é `0 dBFS`;
um pico fornecido acima dele ou EQ positivo sem preamp automático é sinalizado.
O ledger nunca recomenda volume acima de 100%.

Sem dados expostos, ganho/normalização do convolver ou HRIR, ganho/SPL do
endpoint, ganho de resampler/conversão e efeito do dial Game/Chat são sempre
`indeterminado`. Uma diferença de formato/taxa é reportada como divergência,
não como causa ou perda de qualidade. Esses resultados delimitam as medições
A/B necessárias; não autorizam alteração de HID, roteamento, EQ, espacial ou
volume.

## Protocolo A/B pré-registrado (ag04)

O roteiro manual, as métricas, aleatorização, restauração e bloqueios para a
próxima fase ativa estão em [audio-gain-ab.md](audio-gain-ab.md). Ele começa
com baseline passivo e é apenas documentação nesta fase: pares que envolvam
interação física ou mudança de áudio só podem ocorrer após aprovação humana
explícita. O manifesto-modelo é propositalmente não reproduzível e não contém
sinal, comandos ou controles executáveis.
