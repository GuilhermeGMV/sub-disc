# Contrato HTTP e eventos v1

Todas as rotas usam `/api`. Respostas JSON; falhas de aplicação têm `{"error":"mensagem"}`. Rejeições de JSON/esquema do Axum podem retornar texto; o frontend trata ambos. Toda mutação exige `Origin` igual a `APP_ORIGIN`. Autenticação via cookie `subdisc_session`, nunca por ID de usuário fornecido pelo cliente.

| Método e rota | Corpo/consulta | Acesso |
|---|---|---|
| GET `/health` | Verifica SQLite; status e versão | Público |
| GET `/setup` | Retorna `needs_setup` | Público |
| POST `/setup` | `username`, `password`, `setup_key` | Somente instância vazia |
| POST `/login` | `username`, `password` | Público, limitado |
| POST `/register` | `username`, `password`, `invite` | Convite válido |
| POST `/logout` | `{}` | Sessão |
| GET `/me` | Usuário da sessão | Sessão |
| POST `/invites` | `hours`: 1–168; retorna token e URL | Administrador |
| GET `/users` | IDs opacos, nomes, papéis | Sessão |
| GET `/state` | Usuários, canais, online e voz | Sessão |
| GET/POST `/channels` | POST: `name`, `kind`: text/voice | Membro lê, administrador cria |
| PATCH `/channels/:id` | `name`, `kind` existente | Administrador |
| DELETE `/channels/:id` | Remove histórico e sala SFU | Administrador |
| GET `/channels/:id/messages` | `before`: ID exclusivo, `limit`: 1–100, padrão 50 | Sessão, canal de texto |
| POST `/channels/:id/messages` | `content`: 1–4000 caracteres | Sessão, canal de texto |
| POST `/channels/:id/join` | `{}`; retorna URL, token, room, lease | Sessão, canal de voz |
| POST `/media/leave` | `lease` emitida para essa sessão | Sessão |
| GET `/media/ws` | Upgrade, cookie e Origin; primeiro evento `hello` | Sessão e lease |
| POST `/media/publish` | `lease`, `source`, `sdp` (offer) | Sessão dona do lease conectado |
| POST `/media/subscribe` | `lease`, `publication`, `sdp` (offer); opcionais `rid`, `replace` | Mesma sala da publicação |
| POST `/media/control` | `lease`, `id` da assinatura, `paused` | Somente o dono da assinatura |
| POST `/media/close` | `lease`, `id` do transporte | Somente o dono do transporte |
| GET `/ws` | Upgrade, cookie e Origin válidos | Sessão |

Histórico é consultado por cursor de ID, sem OFFSET; o resultado vem em ordem cronológica. `created_at` é Unix em segundos. Usuários e canais usam UUID aleatório. Mensagens têm ID inteiro monotônico. Não é permitido mudar o tipo de um canal que já existe.

Exemplos de eventos do servidor:

```json
{"v":1,"type":"message","message":{"id":42,"channel_id":"uuid","user_id":"uuid","username":"ana","content":"oi","created_at":1792360000}}
{"v":1,"type":"presence","online":["uuid"],"voice":[]}
{"v":1,"type":"channels_changed"}
{"v":1,"type":"users_changed"}
{"v":1,"type":"voice_reset"}
```

Ao conectar e quando o consumidor fica atrasado, o servidor envia `{"v":1,"type":"snapshot","data":{...}}`, com o mesmo formato de `/state`. O cliente recarrega o histórico recente e pode paginar para trás; mensagens são reconciliadas por ID, sem duplicar POST e WebSocket. A reconexão tem backoff e jitter, limitado a 15 segundos.

Único evento permitido do cliente:

```json
{"v":1,"type":"media","lease":"uuid","muted":false,"camera":true,"screen":false}
```

A lease deve pertencer ao usuário e à sessão autenticados. Campos desconhecidos ou versão incompatível encerram a conexão. Estados idênticos não são reenviados pelo cliente a cada mudança de falante. Mídia não trafega por esse socket. Presença online conta conexões de todas as abas; fechar uma delas não coloca offline uma conta que mantém outra aberta. Ping a cada 25 segundos, timeout de 60 segundos, revalidação periódica da sessão e revogação imediata após logout. Hashes de sessão nunca são serializados em eventos públicos.

## Sinalização de mídia

`join` retorna `{url:"/api/media/ws",token,lease,room,iceServers}`. O token opaco
é aleatório (256 bits), armazenado somente como hash, vinculado ao cookie da
sessão e ao lease. A entrada inicial expira após 60 segundos. O cookie também
é obrigatório nos upgrades e em cada oferta HTTP; o token sozinho não autoriza
nenhuma operação. Um lease por conta, até 20 participantes por canal.

O primeiro evento em `/media/ws` é `{type:"hello",lease,token}`. O servidor envia
snapshots `{type:"snapshot",identity,peers,publications}`; cada publicação contém
`id`, `identity`, `source`, `layers` (`[{rid}]`) e `active_layers`. Somente `microphone`, `camera`, `screen_share` e
`screen_share_audio` são aceitas. Opus para áudio e VP8 para vídeo. Um transporte
por fonte publicada e outro por assinatura, sem renegociação de faixas alheias.

Eventos de controle usam `{type:"state",muted,camera,screen,screen_audio_muted}`.
Estes campos são indicadores; mídia continua criptografada em transportes
WebRTC separados. Não há transporte de mídia pelo WebSocket. Cada oferta HTTP
retorna `{id,type:"answer",sdp}` com candidatos ICE completos, sem trickle.

Vídeo pode ser publicado em simulcast com até três `a=rid:<id> send`
alfanuméricos (o navegador usa `l`, `m` e `h`); áudio usa uma camada. Sem `rid`, a
assinatura recebe a última camada oferecida. Para trocar de camada sem
interromper a imagem, o cliente abre uma assinatura com `replace` igual ao `id`
da atual e fecha a anterior quando a nova reproduz. `active_layers` lista as
camadas com assinaturas não pausadas; o publicador desativa as demais.
`/media/control` com `paused:true` suspende o encaminhamento de uma assinatura
fora da tela; ao retomar, o SFU pede um keyframe. Ofertas: 240 por minuto por conta.

Uma interrupção da sinalização reserva o lease por 60 segundos; os clientes
reconectam por até 45 segundos. O novo socket substitui o anterior usando uma
época interna; timers de sockets antigos não revogam a conexão nova. Logout,
remoção da sala e saída explícita fecham publicações e assinaturas dependentes.
Reiniciar o backend invalida todos os leases: o cliente informa que é preciso
entrar novamente. Limite JSON: 64 KiB; sinalização WS: 2 KiB por evento.
