// Two Steam accounts on two machines: one hosts a room, the other finds it and joins. Covers what a
// room needs: a hidden lobby found by a filtered worldwide search, lobby data, ownership moving when
// the host leaves, and relay-only messages both ways, unreliable and reliable.
//
//   node test/rooms.js <appId> host
//   node test/rooms.js <appId> join
//
// Rooms carry a namespace no game uses, so this never finds a player's room or is found by one.
// Ctrl+C leaves the lobby; stop the host first to watch ownership move to the joiner.

const { init, SteamCallback } = require('../index.js')

const [appId, role] = process.argv.slice(2)
if (!appId || !['host', 'join'].includes(role)) {
    console.log('usage: node test/rooms.js <appId> host|join')
    process.exit(1)
}

const NS = 'koyuuji-rooms-test'
const client = init(Number(appId))
const { matchmaking, messages, localplayer } = client
const me = localplayer.getSteamId().steamId64

const UNRELIABLE = messages.SendFlags.Unreliable | messages.SendFlags.AutoRestartBrokenSession
const RELIABLE = messages.SendFlags.Reliable | messages.SendFlags.AutoRestartBrokenSession

console.log('relay only:', messages.setRelayOnly())
messages.initRelayNetworkAccess()
messages.onSessionFailed((failure) => console.log('session failed:', failure))

let lobby = null
const peers = () => lobby.getMembers().map((m) => m.steamId64).filter((id) => id !== me)

client.callback.register(SteamCallback.LobbyChatUpdate, (update) => {
    console.log('lobby membership changed:', update)
    if (lobby) messages.acceptSessionsFrom(peers())
})

async function host() {
    lobby = await matchmaking.createLobby(matchmaking.LobbyType.Invisible, 4)
    lobby.mergeFullData({ ns: NS, born: String(Math.floor(Date.now() / 1000)) })
    console.log(`hosting ${lobby.id}; waiting for the joiner`)
}

async function join() {
    for (;;) {
        const found = await matchmaking.getLobbies({
            strings: [{ key: 'ns', value: NS }],
            distance: matchmaking.LobbyDistance.Worldwide,
            count: 10,
        })
        console.log(`search found ${found.length}`, found.map((l) => `${l.id} born=${l.getData('born')}`))
        if (found.length > 0) {
            lobby = await found[0].join()
            console.log(`joined ${lobby.id}; owner ${lobby.getOwner().steamId64}`)
            return
        }
        await new Promise((resolve) => setTimeout(resolve, 3000))
    }
}

const received = new Map()
let sequence = 0

function run() {
    messages.acceptSessionsFrom(peers())

    // Ten unreliable frames a second, as a room would send positions.
    setInterval(() => {
        const frame = Buffer.alloc(4)
        frame.writeUInt32LE(sequence++)
        for (const peer of peers()) messages.sendToUser(peer, frame, UNRELIABLE, 0)
    }, 100)

    setInterval(() => {
        for (const peer of peers()) messages.sendToUser(peer, Buffer.from(`hello from ${role}`), RELIABLE, 1)
    }, 2000)

    setInterval(() => {
        for (const channel of [0, 1]) {
            for (const message of messages.receive(channel, 64)) {
                const from = message.steamId.steamId64.toString()
                const entry = received.get(from) ?? { frames: 0, last: '' }
                if (channel === 0) entry.frames++
                else entry.last = message.data.toString()
                received.set(from, entry)
            }
        }
    }, 50)

    setInterval(() => {
        const relay = messages.relayStatus()
        console.log(
            `relay ${relay.availability} (any relay ${relay.anyRelay})`,
            `| owner ${lobby.getOwner().steamId64 === me ? 'me' : lobby.getOwner().steamId64}`,
            `| peers ${peers().length}`,
            '| received',
            Object.fromEntries(received),
        )
        received.forEach((entry) => (entry.frames = 0))
    }, 5000)
}

process.on('SIGINT', () => {
    if (lobby) lobby.leave()
    console.log('left')
    setTimeout(() => process.exit(0), 500)
})

;(role === 'host' ? host() : join()).then(run).catch((error) => {
    console.error(error)
    process.exit(1)
})
