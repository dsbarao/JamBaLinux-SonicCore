# Handoff: recuperar o equalizador do JamBaLinux

Você recebeu este handoff porque a implementação atual do equalizador foi
entregue sem a validação necessária. Assuma a responsabilidade técnica pela
recuperação, não declare sucesso com base apenas em testes unitários, e não
mascare os problemas abaixo.

## Objetivo do usuário

Entregar um equalizador de 10 bandas para o headset JBL Quantum 810 no
JamBaLinux, com as frequências 31, 62, 125, 250, 500 Hz, 1, 2, 4, 8 e 16 kHz.
Ele deve afetar somente a rota **Game**, preservar Chat e microfone, funcionar
automaticamente quando o headset for usado e ter uma ação "Zerar bandas" que
atualize o áudio e a interface imediatamente. O usuário não quer um botão
manual "Ativar no PipeWire".

## Falhas comprovadas

- O widget persiste e exibe ganhos, mas "Zerar bandas" não atualiza
  confiavelmente a interface.
- Mudar qualquer banda interrompe ou pausa vídeos do navegador.
- O backend atual executa `systemctl --user restart` no filtro PipeWire a cada
  `equalizer set` e `equalizer reset`; isso recria nós/links e é a causa mais
  provável da interrupção.
- O sink virtual `JamBaLinux Game Equalizer` foi criado, porém ele acabou como
  sink padrão em pelo menos um estado real. Isso não atende à promessa de
  preservação da rota padrão e não pode ser aceito sem uma política explícita
  e reversível.
- Uma tentativa de atualizar os controles internos via `pw-cli set-param` não
  alterou o ganho. Portanto, não há API de mutação em tempo real validada para
  a arquitetura atual com `libpipewire-module-filter-chain`.

## Evidência de rota observada

Em uma verificação real havia Chrome -> sink virtual EQ -> saída do EQ -> JBL
Game, sem links para Chat. Os nós físicos do Quantum foram identificados por
`alsa.components = USB0ecb:2069`: device 0 é Game e device 1 é Chat. Não altere
VID/PID, allowlists ou protocolo HID.

## Arquivos relevantes

- `src/pipewire.rs`: descoberta de sinks e geração/controle do filtro atual.
- `src/equalizer.rs`: perfil persistido e comandos CLI.
- `src/main.rs`: hoje aciona reload/restart após alterações — este é o caminho
  que precisa deixar de derrubar a cadeia.
- `packaging/plasma/org.jambalinux.soniccore/contents/ui/main.qml`: controles,
  estado do widget e reset.
- `packaging/systemd/jambalinux-soniccore-equalizer.service`.
- `docs/protocol/audio-processing.md` e `AGENTS.md` devem ser lidos antes de
  mudar comportamento.

## Direção arquitetural exigida

Não faça um restart por slider. Uma solução aceitável exige cadeia persistente
e atualização dinâmica/atômica dos coeficientes por bloco, com rampa curta
(20–50 ms), usando um daemon DSP próprio ou outra API comprovadamente mutável.
Somente após a mutação em tempo real estar demonstrada, trate a interface como
funcional.

O roteamento deve ser automático e reversível: processe somente streams que
estavam explicitamente na rota Game ou cujo destino tenha sido comprovado;
registre identificador/serial e destino original para restauração. Não capture
Chat, microfone ou todos os streams globais, e não force o sink virtual como
padrão sem consentimento explícito do usuário.

## Critérios de aceite — todos obrigatórios

1. Com um vídeo tocando no navegador, alterar 500 Hz e 16 kHz não pausa,
   reinicia ou desconecta o stream; confirme isso com observação real e com o
   estado PipeWire antes/depois.
2. O ganho aplicado muda de fato no DSP sem `systemctl restart`, sem recriar
   o sink e sem links transitórios para Chat.
3. "Zerar bandas" zera as 10 bandas no DSP, no perfil salvo e no widget na
   mesma ação; só anuncie sucesso após confirmar os três.
4. Game é processado; Chat e microfone permanecem fora da cadeia. O estado
   deve sobreviver a reconexão do headset de forma previsível.
5. Não existe botão de ativação manual como pré-requisito de uma função que
   deveria operar automaticamente. Caso haja indisponibilidade do PipeWire,
   mostre um erro acionável em vez de simular estado ativo.

## Processo de trabalho

Leia `README.md`, `docs/rebranding.md`, o protocolo de áudio e `AGENTS.md`.
Inspecione primeiro o worktree sujo e preserve alterações alheias. Faça uma
proposta curta da arquitetura e implemente apenas após ela ser tecnicamente
viável. Rode testes Rust, clippy, qmllint e, principalmente, as verificações
reais do PipeWire descritas acima. Relate quais foram executadas, seus comandos
e resultados; se algum critério não for verificável, declare a entrega
bloqueada em vez de afirmar que funciona.
