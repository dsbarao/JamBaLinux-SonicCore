# Validação de hardware — spatial binaural

**Estado:** aprovado com fixture sintética; HRIR de distribuição continua sujeita a revisão jurídica; transição persistente/bypass requer aceite auditivo posterior
**Data do procedimento:** 2026-09-20  
**Responsável presente:** mantenedor do projeto  
**Versão/commit testado:** `7338ccb` (`migration/jambalinux`)

Este é o protocolo e o registro de aceite da fase 7. A sessão foi executada na
máquina do mantenedor, com o JBL Quantum 810 Wireless presente. A fixture HRIR
foi gerada localmente e serve somente para validar roteamento, convolução e
estabilidade; ela não avalia a qualidade perceptiva de uma HRIR binaural real.

## Limites de segurança e preparação

O teste é opt-in e usa exclusivamente o modo aberto `binaural-stereo`. Não
envia relatórios USB/HID, não muda VID/PID, allowlists, firmware ou a saída
padrão. A cadeia autorizada é:

```text
aplicação 7.1 -> jambalinux-soniccore-game-spatial
               -> jambalinux-soniccore-game-equalizer
               -> Quantum Game físico -> headset
```

Chat e qualquer nó de captura, inclusive o microfone, devem ficar fora dessa
cadeia. Não usar arquivos, nomes ou alegações de interoperabilidade de
fornecedores. EVT-035 e EVT-036 já estabeleceram que o processamento espacial
é de host e não requer comando USB; esta validação não altera essa conclusão.

Antes de ativar, o mantenedor deve confirmar que entende que o sink **JamBaLinux
Game Spatial** é selecionado apenas no jogo/aplicação de teste; ele não deve
virar a saída padrão. Salve o perfil de EQ atual para restaurá-lo ao final.

## Registro de baseline (somente leitura)

Com o dongle conectado e antes de colocar qualquer aplicação no sink espacial,
registre a saída integral dos comandos abaixo na sessão de validação. Eles só
consultam o estado do usuário, do PipeWire e dos serviços.

```bash
soniccore spatial preflight --format json
soniccore spatial status --format json
soniccore equalizer status --format json
pactl get-default-sink
pactl list short sinks
pactl list short sources
pw-dump
systemctl --user status jambalinux-soniccore-equalizer.service
systemctl --user status jambalinux-soniccore-spatial.service
```

| Evidência de baseline | Resultado |
|---|---|
| Saída padrão antes do teste | `alsa_output.usb-Harman_International_Inc_JBL_Quantum810_Wireless-00.stereo-game-output` |
| Nó/serial do Game físico e do EQ | Game serial `70075`; o EQ permaneceu único durante cada amostra (o processo foi reinstalado durante as correções) |
| Perfil de EQ antes do teste | Custom: `31:+3, 62:+3, 125:+3, 250:0, 500:-2, 1k:-2, 2k:-2, 4k:+7, 8k:+10, 16k:+12` dB |
| Serviços EQ e Spatial ativos/estado | Ambos ativos; preflight pronto e status sem erro após as correções de runtime |
| Links ativos relevantes | Spatial FL/FR -> EQ FL/FR -> Quantum Game FL/FR; nenhum link direto Spatial -> Game |
| Chat, fonte de microfone e saída padrão fora da cadeia | Confirmado no baseline, durante 10 minutos e após reconexão |

## HRIR sintética controlada

Comece com uma HRIR sintética local de 14 canais; ela é evidência de
roteamento, não de qualidade binaural. Gere-a localmente durante a sessão,
com licença declarada como `CC0-1.0` para o próprio sinal criado. O comando
abaixo cria somente os dois arquivos de teste exigidos pelo contrato em
`$XDG_CONFIG_HOME/jambalinux-soniccore/spatial/hrtf/` (ou em
`~/.config/...` quando `XDG_CONFIG_HOME` não estiver definido). Não baixa,
redistribui nem sobrescreve arquivos sem uma cópia de segurança prévia.

```bash
python3 - <<'PY'
import hashlib, json, os, struct, wave

root = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
directory = os.path.join(root, "jambalinux-soniccore", "spatial", "hrtf")
os.makedirs(directory, exist_ok=True)
path = os.path.join(directory, "hrir.wav")
if os.path.exists(path) or os.path.exists(os.path.join(directory, "manifest.json")):
    raise SystemExit("recusado: preserve ou remova manualmente o dataset existente antes do teste")

# 14 canais PCM/48 kHz. Cada resposta tem impulso de amplitude e atraso
# distintos, para permitir correlação inequívoca de cada convolver.
frames = 4096
payload = bytearray()
for frame in range(frames):
    for channel in range(14):
        delay = channel * 37
        sample = (30000 - channel * 1000) if frame == delay else 0
        payload.extend(struct.pack("<h", sample))
with wave.open(path, "wb") as wav:
    wav.setnchannels(14)
    wav.setsampwidth(2)
    wav.setframerate(48000)
    wav.writeframes(payload)
digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
manifest = {
    "schema": 1,
    "format": "surround-7.1-14ch-wav",
    "name": "Synthetic 14-channel impulse routing fixture",
    "sha256": digest,
    "license": "CC0-1.0 (generated locally for validation)",
    "source_url": "https://github.com/dsbarao/JamBaLinux-SonicCore",
}
with open(os.path.join(directory, "manifest.json"), "x", encoding="utf-8") as out:
    json.dump(manifest, out, indent=2)
    out.write("\n")
print(path)
print(digest)
PY
soniccore spatial preflight --format json
```

O preflight deve mostrar `dataset.valid: true`, 14 canais, 48000 Hz e hashes
esperado/observado idênticos. A licença dessa fixture só cobre o sinal gerado
na sessão. Para uma HRIR aberta real, pare e obtenha a revisão humana da
licença e proveniência indicada em `manifest.json` antes de usá-la; validade
técnica não equivale a autorização legal.

| Verificação técnica da fixture | Resultado |
|---|---|
| `dataset.valid`, 14 canais e 48000 Hz | Aprovado |
| SHA-256 esperado = observado | `549308958f8ad7610da25795494cdb7ff1ac73488956ecffea6e16354c483c93` |
| Sem arquivo preexistente sobrescrito | Confirmado; diretório estava livre para a fixture da sessão |
| Revisão de licença/proveniência para HRIR aberta não sintética | Não aplicável à fixture CC0 gerada localmente; continua obrigatória antes de distribuir uma HRIR real |

## Ativação e prova por canal

1. Confirme que `jambalinux-soniccore-game-equalizer` existe exatamente uma
   vez. Execute `soniccore spatial mode binaural-stereo` e `soniccore spatial
   enable`.
2. Aguarde o supervisor e registre `soniccore spatial status --format json`.
   Só avance se `active`, `input_format_7_1`, `output_format_stereo`,
   `target_equalizer_connected`, `default_safe`, `chat_isolated`,
   `capture_isolated` e `routing_healthy` forem `true`.
3. Abra uma aplicação/sinal de teste explicitamente no sink **JamBaLinux Game
   Spatial**, negociada em 7.1. Não mova Chat, microfone nem a saída padrão.
4. Injete um impulso ou tom curto em um canal por vez na ordem `FL`, `FR`,
   `FC`, `LFE`, `RL`, `RR`, `SL`, `SR`. Faça uma captura digital do output
   estéreo antes do headset, ou uma medição equivalente que preserve amostras.
   Para cada entrada, correlacione a captura com os 14 impulsos distintos da
   fixture: registre os dois índices HRIR esperados, os atrasos e os picos. A
   escuta humana é complementar e não substitui correlação.

| Entrada 7.1 | Índices HRIR/canais esperados pelo grafo (L, R) | Atrasos/picos medidos na captura estéreo | Correlação aprovada | Audição confirmada |
|---|---|---|---|---|
| FL | 0, 1 | L `99200/4578`; R `99237/4425` (R +37 amostras) | Sim | Sim |
| FR | 8, 7 | L `147496/3357`; R `147459/3510` (L +37) | Sim | Sim |
| FC | 6, 13 | L `195422/3662`; R `195681/2594` (R +259) | Sim | Sim |
| LFE | 6, 13 (tratado como FC no MVP) | L `243422/3662`; R `243681/2594` (R +259) | Sim | Sim |
| RL | 4, 5 | L `291348/3967`; R `291385/3815` (R +37) | Sim | Sim |
| RR | 12, 11 | L `339644/2747`; R `339607/2899` (L +37) | Sim | Sim |
| SL | 2, 3 | L `387274/4272`; R `387311/4120` (R +37) | Sim | Sim |
| SR | 10, 9 | L `435570/3052`; R `435533/3204` (L +37) | Sim | Sim |

Os valores acima são `frame/pico` em PCM S16. Além dos atrasos relativos, as
razões entre picos coincidiram com as amplitudes únicas da fixture. O
mantenedor ouviu os oito sinais sequenciais no headset antes e depois da
reconexão. A coluna de audição confirma presença dos oito canais; a separação
L/R foi comprovada pela captura digital, não por uma avaliação subjetiva de
localização.

Reprove o teste se qualquer canal chegar diretamente ao headset físico, a
Chat/captura, ou se aparecer ligação adicional ao destino do output espacial.

## Cadeia, EQ e estabilidade

Durante uma reprodução 7.1 contínua, capture `pw-dump` e confirme os links
`aplicação -> spatial -> EQ -> Game`. Registre também o JSON de status dos
dois componentes. Mude apenas uma banda do EQ que estava no perfil registrado
(por exemplo, 1 kHz em +6 dB), meça o nível no output pós-binauralização e
restaure imediatamente o valor original. A alteração só é aprovada se o nível
medido mudar de forma coerente e o perfil original for restaurado.

Mantenha o sinal em execução por 10 minutos. Registre CPU e xruns com uma
ferramenta local disponível (por exemplo, `pw-top` em modo batch e `pidstat`
para os PIDs de `pipewire` e do supervisor espacial), além de uma amostra
inicial e final de `soniccore spatial status --format json`. Não declare zero
xruns sem uma leitura da coluna/contador correspondente.

| Critério contínuo | Resultado/medição |
|---|---|
| Links aplicação -> Spatial -> EQ -> Game observados | Sim, no `pw-link`, `pactl` e status JSON |
| Alteração de EQ e diferença pós-binaural medida | 1 kHz: −2 -> +6 dB; RMS L `1205,913 -> 3028,958`, R `1165,621 -> 2927,744` (razão `2,512x`, equivalente a +8 dB) |
| Perfil de EQ original restaurado | Sim; `profile_synced=true`, 1 kHz novamente em −2 dB |
| Duração contínua (mínimo 10 min) | 605 segundos |
| CPU do PipeWire/supervisor (média e pico) | DSP spatial `0,300% / 0,400%`; DSP EQ `0,257% / 0,300%`; supervisor spatial `2,436% / 2,500%`; supervisor EQ `0,100% / 0,100%`; PipeWire principal `0,600% / 0,600%` (121 amostras) |
| Xruns (contador/fonte) | `pw-top` coluna `ERR`: máximo 0 em 10.138 linhas de nós, nenhuma linha positiva |
| Chat inalterado | Sim; isolado em todas as amostras de saúde |
| Microfone/captura inalterados | Sim; isolado em todas as amostras de saúde |
| Saída padrão inalterada | Sim; Quantum Game antes e depois |
| Falhas, dropouts ou latência percebida | Nenhum dropout relatado; continuidade do fallback confirmada. Latência subjetiva não foi quantificada separadamente |

## Fallback e reconexão física

Com reprodução ativa, execute `soniccore spatial disable`. Confirme que os
streams ainda ligados ao spatial foram encaminhados ao EQ comprovado, que o
grafo espacial desapareceu e que o áudio continua em estéreo de forma
controlada. Reative somente após registrar o resultado.

Para a reconexão, deixe o serviço Spatial habilitado, desconecte fisicamente o
dongle USB, aguarde o Game físico desaparecer, reconecte-o e espere a
recuperação automática. Não reinicie PipeWire, WirePlumber, EQ nem qualquer
serviço para mascarar o resultado. Registre IDs/serials antes/depois e confirme
que há apenas um input e um output espacial, ambos saudáveis, com a cadeia
recomposta até o novo Game físico.

| Recuperação | Resultado |
|---|---|
| Desativação encaminhou streams sem interromper Chat/mic/padrão | Sim; o mesmo stream `556651` foi transferido ao EQ `554703` e a reprodução continuou até o fim |
| Dongle removido e reconhecido fisicamente | Sim, pelo mantenedor presente |
| Serviço permaneceu ativo ou se recuperou por conta própria | EQ e Spatial ativos, `NRestarts=0`; nenhum PipeWire/WirePlumber/serviço foi reiniciado para a recuperação |
| Grafo recriado sem nós espaciais duplicados | Sim; exatamente 1 input/output Spatial e 1 input/output EQ |
| Novo serial do Game e links refeitos até o headset | Game `70075 -> 555478`; EQ -> novo Game e Spatial -> EQ confirmados |
| Reprodução pós-reconexão confirmada auditivamente | Sim; sequência de oito sinais ouvida pelo mantenedor |

## Encerramento e decisão

1. Desative o spatial e confirme fallback seguro.
2. Restaure o perfil de EQ e a saída padrão registrados no baseline, caso
   tenham sido modificados fora do comportamento esperado.
3. Preserve os valores desta tabela e anexe somente saídas sanitizadas, sem
   dados pessoais nem capturas USB brutas.

| Aceite | Decisão |
|---|---|
| Todos os oito canais correlacionados e auditivamente confirmados | APROVADO |
| Cadeia e efeito do EQ comprovados | APROVADO |
| 10 minutos sem interferência em Chat/mic/padrão | APROVADO |
| CPU/xruns dentro do limite aceito pelo mantenedor | APROVADO com as medições acima |
| Reconexão física recuperou o grafo | APROVADO |
| Aprovação final do mantenedor | APROVADO presencialmente em 2026-09-20 |

O spatial ficou desativado ao final, o perfil de EQ original permaneceu
sincronizado e a saída padrão continuou no Quantum Game. Este aceite não
substitui a revisão legal de uma HRIR aberta de terceiros nem constitui uma
avaliação de qualidade binaural da fixture sintética.
