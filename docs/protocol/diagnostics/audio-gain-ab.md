# Protocolo A/B pré-registrado — ganho de áudio

**Estado:** protocolo manual, reversível e não executado. Este arquivo não autoriza coleta ativa, alteração de áudio ou interação física.  
**Depende de:** o contrato passivo em [audio-gain.md](audio-gain.md) e de uma aprovação humana explícita, por sessão, antes do primeiro par A/B.  
**Caminho em estudo:** `aplicação -> Spatial -> EQ -> Quantum Game`; Chat é medido como endpoint de comparação, nunca incluído no caminho processado.

## Propósito e fronteiras

Este protocolo distingue, por medição digital e por uma avaliação humana separada, uma perda de ganho de: bypass/binaural, downmix, HRIR/convolvers, EQ, endpoints Game/Chat e conversão/resampling. Ele não presume que qualquer um seja a causa do relato.

Nesta fase, não executar os pares. Em especial, não tocar na roda, não trocar rota, Spatial ou EQ, não mudar volume, não iniciar/parar serviço e não enviar ou solicitar relatório HID. A posição física é apenas uma anotação manual; os bytes HID, VID/PID, allowlists e regras de protocolo não fazem parte do experimento. Uma hipótese sobre HID continua proibida até validação explícita em hardware e revisão própria.

Quando e somente quando a aprovação humana existir, o operador pode executar o roteiro abaixo manualmente. Cada mudança ativa deve ser uma ação já exposta pela interface regular, uma por vez, e deve ser desfeita antes da condição seguinte. Não introduzir comandos, relatórios, valores, links ou caminhos de controle novos para este diagnóstico.

## Pré-registro da sessão

Antes de sortear uma condição, preencher uma cópia de `tests/fixtures/audio_gain/ab-protocol-manifest-template.json` fora do repositório. O modelo é deliberadamente não reproduzível: não contém sinal de teste, áudio, comandos, valores de controle, dados de pessoa ou identificação de hardware. Ele só fixa os campos que devem ser registrados.

Anotar o seguinte baseline passivo antes e depois de cada par:

| Campo | Registro exigido | Interpretação proibida |
|---|---|---|
| Fonte | identidade e hash do mesmo arquivo/sinal, trecho, duração, janela e nível digital da aplicação | que volume de aplicação equivale a SPL |
| Rota | stream, destino, `id` e `object.serial`, links e caminho observado | que um ID persistirá após recriação |
| Níveis efetivos | volume/mute linear e dB quando exposto para stream e sink; Base Volume; posição física anotada | que controle ou posição é nível de amostra |
| Grafo | `pw-dump`, formatos/taxas/mapas negociados por link, Props wet/dry, bandas EQ e hash/manifesto HRIR | que propriedade declarada é formato negociado |
| Capturas | ponto pré/pós, PCM/float, full scale, taxa, canais, ordem, ferramenta, resolução e relógio | que duas capturas sem alinhamento são comparáveis |

O baseline é válido apenas se a mesma fonte, nível de aplicação, posição física e volumes efetivos forem mantidos dentro do par. A rota também deve permanecer igual, exceto no par Endpoint, em que Game/Chat é precisamente a única variável: aí a rota até o endpoint e o endpoint selecionado devem ser anotados em A e B. Se qualquer outra variável mudar, anular o par, restaurar o estado anotado e coletar um novo baseline. Para cada condição, guardar somente dados sanitizados; nomes de aplicação/mídia podem ser pessoais.

## Sinal, capturas e métricas comuns

Usar uma referência digital idêntica em A e B. O conjunto pré-registrado deve conter, no mínimo: tom estável para nível, impulsos ou ruído codificado para alinhamento/correlação, e estímulos isolados FL, FR, FC, LFE, SL, SR, RL e RR para atribuição de canais. O sinal não é fornecido por este repositório e não deve ser improvisado depois de ver o resultado.

Capturar digitalmente, quando o ponto for observável, antes e depois do estágio sob teste. Alinhar pelo estímulo de referência e declarar atraso. Medir por canal e por janela igual:

- RMS e pico em dBFS, com `full_scale` declarado; true-peak separado se suportado.
- Correlação normalizada, atraso do máximo e crosstalk em dB entre cada entrada 7.1 e cada saída L/R.
- Formato, taxa, canais e ordem efetivamente negociados em cada link, além do formato da captura.
- Diferença A-B de RMS/pico e, para o downmix, soma prevista versus observada.

Para tom estável na mesma cadeia, aceitar até 0,2 dB de diferença RMS/pico entre repetições e até uma amostra de variação no atraso esperado. Fora disso, repetir e marcar inconclusivo. Para música ou fala, reportar distribuição ou intervalo de repetições após alinhamento; não aplicar a tolerância do tom. Pico de amostra ou true-peak acima de 0 dBFS torna a condição sem headroom demonstrado, mesmo que não haja distorção audível.

## Desenho experimental e aleatorização

Cada comparação usa pares A/B com uma única variável deliberadamente alterada. Pré-registrar uma semente ou método físico de sorteio antes da sessão e gerar, para cada família, ordem balanceada `AB/BA` em pelo menos três repetições. Intercalar uma repetição de baseline entre famílias. O anotador não deve escolher a ordem após ouvir ou inspecionar métricas.

O avaliador humano, se presente, recebe pares nivelados apenas quando isso não mascara a medição de ganho: guardar a versão de nível nativo como resultado primário e, opcionalmente, uma versão nivelada apenas para qualidade/timbre. A avaliação subjetiva (preferência, inteligibilidade, espacialidade e escala) nunca substitui RMS, pico, correlação, formato ou verificação de clipping.

## Pares A/B obrigatórios

| Família | A e B, com todo o resto constante | Evidência objetiva mínima | Resultado que discrimina, sem atribuir causa além dos dados |
|---|---|---|---|
| Spatial | bypass seco (wet=0/dry=1) versus binaural (wet=1/dry=0) | RMS/pico pré/post, true-peak quando disponível, correlação/atraso por entrada e L/R, Props lidos e formatos/taxas | diferença estável só neste par localiza a investigação no caminho wet/HRIR, não prova normalização HRIR |
| EQ | Flat (dez bandas 0 dB) versus um perfil pré-registrado | bandas solicitadas e lidas, RMS/pico/true-peak pré/post por tom e faixa, formatos/taxas e ausência de clipping | alteração além da tolerância com controles confirmados caracteriza efeito DSP; clipping bloqueia compensação de ganho |
| Layout | estéreo FL/FR versus 7.1 com estímulos por canal | correlação para todos os oito canais, matriz e soma observada, RMS/pico L/R, LFE explicitamente verificado | canal ausente, mapeado errado ou soma divergente bloqueia alteração de coeficiente; estéreo não prova o upmix 7.1 |
| Endpoint | Quantum Game versus Quantum Chat | rota/serial de ambos, stream e sink volume, RMS/pico, formato/taxa/mapa e resampling exposto | diferença de endpoint é registrada como tal; não prova que a roda altere ganho ou SPL |
| Roda física (observacional) | posição anotada no início do par versus a mesma posição no fim | rótulo Game/Chat/centro, fotos/anotação do detente se consentido, volumes e endpoints passivos | divergência torna o par inválido; não se lê nem se escreve HID e não se infere efeito de ganho |

No par de layout, testar primeiro cada canal isolado a uma amplitude segura e constante, depois combinações correlacionadas FC+FL+SL+RL e FC+FR+SR+RR. A referência seca documentada prevê L=`FL + .707 FC + .707 SL + .707 RL` e R=`FR + .707 FC + .707 SR + .707 RR`, com LFE omitido. Uma soma correlacionada pode alcançar `1 + 3×0.707 = 3.121` (cerca de +9,887 dB), portanto nunca se interpreta 0,707 como garantia de headroom.

## Verificações específicas de HRIR, conversão e rampas

O par Spatial deve registrar hash/manifesto, taxa, canais, duração e RMS/pico de cada resposta HRIR disponível, além dos 16 caminhos de convolver. Sem isso, o resultado pode indicar diferença entre wet e dry, mas a normalização/ganho HRIR permanece indeterminada. LFE que reutiliza resposta de FC é topologia documentada, não equivalência de nível.

Confrontar o formato de fonte, de cada link, de cada captura e de cada endpoint. Qualquer mudança de PCM/float, taxa, número/ordem de canais, remapeamento ou resampling exposto deve ser marcada como confundidor. Repetir com formato/taxa compatíveis quando possível; se não for, registrar a divergência e não chamá-la de perda de qualidade ou de ganho.

Registrar os quatro Props wet/dry antes e após cada condição e o intervalo real observado de qualquer rampa. A rampa documentada é uma intenção de controle, não evidência de valor aplicado no instante da captura. Capturas que intersectam uma transição, Props incompletos ou estado de gate divergente são inválidos.

## Restauração e regras de bloqueio

Depois de cada B e ao encerrar a sessão, restaurar exatamente o baseline anotado: estado Spatial/EQ, rota, destino, volumes/mute, posição física e nível da aplicação. Confirmar a restauração por nova observação passiva de stream, sink, serial, links, formatos, Props, bandas e volumes. Se não puder restaurar ou verificar, interromper a sessão e não continuar com outros pares.

Os seguintes resultados bloqueiam qualquer mudança de DSP, inclusive preamp, ganho, coeficiente, HRIR, roteamento ou compensação de volume:

- clipping ou ausência de headroom demonstrado em qualquer condição;
- fonte, nível, rota, posição física, volumes, janela ou alinhamento não equivalentes dentro de um par;
- formatos/taxas/mapas divergentes não isolados ou resampling não documentado;
- correlação/crosstalk ausente para algum canal 7.1, matriz seca divergente ou LFE não verificado;
- hash/manifesto e níveis HRIR/convolver ausentes;
- Props, bandas, endpoint, serial ou volumes efetivos não observáveis;
- diferenças sem repetição, acima das tolerâncias ou incompatíveis entre AB e BA; e
- qualquer necessidade de tocar em HID, VID/PID, allowlist, bytes de relatório ou controle não previamente autorizado.

Uma mudança só pode ser proposta após uma decisão humana posterior que cite os resultados brutos sanitizados, a análise por repetição e os bloqueios resolvidos. Este protocolo não recomenda volume acima de 100%, nem presume que elevar volume seja seguro ou corrija qualidade.
