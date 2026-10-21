# Contrato HTTP e eventos v1

Todas as rotas usam `/api`. Respostas JSON; falhas de aplicação têm `{"error":"mensagem"}`. Rejeições de JSON/esquema do Axum podem retornar texto; o frontend trata ambos. Toda mutação exige `Origin` igual a `APP_ORIGIN`; a exceção é o webhook assinado do LiveKit. Autenticação via cookie `subdisc_session`, nunca por ID de usuário fornecido pelo cliente.

| Método e rota | Corpo/consulta | Acesso |
|---|---|---|
| GET `/health` | Verifica SQLite; status e versão | Público |
| GET `/setup` | Retorna `needs_setup` | Público |
| POST `/setup` | `username`, `password`, `setup_key` | Somente instância vazia |
| POST `/login` | `username`, `password` | Público, limitado |
| POST `/register` | `username`, `password`, `invite` | Convite válido |
| POST `/logout` | `{}` | Sessão |
| GET `/me` | Usuário da sessão | Sessão |
| POST `/invites` | `{}`; validade fixa de 24 h, até 5 cadastros; retorna `token`, `url`, `expires_at`, `max_uses` e `uses_remaining` | Administrador |
| GET `/users` | IDs opacos, nomes, papéis | Sessão |
| POST `/users` | `username`, `password` (12–128 bytes), `role`: admin/member | Administrador |
| PATCH `/users/:id` | `username`, `role`; `password` opcional/vazia mantém a atual | Administrador |
| DELETE `/users/:id` | Apaga conta, mensagens e convites; revoga sessões e mídia | Administrador |
| POST `/users/:id/disconnect` | `{}`; encerra a chamada e seus transportes | Administrador |
| GET `/state` | Usuários, canais, online e voz | Sessão |
| GET/POST `/channels` | POST: `name`, `kind`: text/voice | Membro lê, administrador cria |
| PATCH `/channels/:id` | `name`, `kind` existente | Administrador |
| DELETE `/channels/:id` | Remove histórico e sala SFU | Administrador |
| GET `/channels/:id/messages` | `before`: ID exclusivo, `limit`: 1–100, padrão 50 | Sessão, canal de texto |
| POST `/channels/:id/messages` | `content`: 1–4000 caracteres | Sessão, canal de texto |
| POST `/channels/:id/join` | `{}`; retorna URL, token, room, lease | Sessão, canal de voz |
| POST `/media/leave` | `lease` emitida para essa sessão | Sessão |
| POST `/livekit/webhook` | Corpo bruto e `Authorization` do SDK | LiveKit autenticado |
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

As chaves LiveKit permitem administração do SFU e devem ser mantidas no servidor. Tokens de entrada aceitam somente microphone, camera, screen_share e screen_share_audio para uma sala; não concedem administração, criação explícita de salas, alteração de metadata ou publicação de dados. A validade do token limita a entrada inicial; reconexões seguem o mecanismo oficial do SDK.

## Administração

O painel **Administração** fica no cliente web servido pelo backend e no cliente
Rust/egui. No navegador, `/admin` também abre o painel após login. Ambos usam as rotas autenticadas acima. Apenas `admin` e `member`
são aceitos, inclusive pelo CHECK do SQLite. O setup cria o primeiro admin;
convites criam membros. Admins podem criar contas diretamente e editar nome,
senha e role. O último administrador não pode ser excluído nem rebaixado;
a verificação ocorre na mesma transação SQLite que a alteração.

Trocar a senha revoga todas as sessões e encerra a mídia da conta. Apagar uma
conta remove também suas mensagens e os convites criados ou usados por ela,
por isso a interface solicita confirmação. Tirar alguém da chamada invalida
seu lease, remove o participante do LiveKit e envia `voice_reset` apenas às conexões
daquela conta. O usuário continua autenticado e pode entrar novamente.

Convites novos expiram exatamente 24 horas após a criação. Usar o link não renova o prazo; somente cadastros concluídos consomem um uso, com limite de cinco inclusive em acessos simultâneos. O campo legado `hours` é aceito, mas ignorado. `uses_remaining` na resposta de criação é 5 (não é um contador atualizado em tempo real). A migração preserva a expiração dos convites anteriores e mantém inválidos os que já haviam sido consumidos.
