import { Channel, Server } from "rjweb-server"
import { Runtime } from "@rjweb/runtime-node"
import { number } from "@rjweb/utils"
import * as fs from "fs"

const seperator = Buffer.from([0x69, 0x62, 0x64, 0x69]),
  options = {
    nosave: process.argv.includes('--nosave')
  }

if (!fs.existsSync('../history.raw') && !options.nosave) fs.writeFileSync('../history.raw', Buffer.concat([Buffer.from([0x01]), seperator]))
const write = !options.nosave ? fs.createWriteStream('../history.raw', {
  flags: 'a'
}) : null

const server = new Server(Runtime, {
  port: process.env.PORT ? parseInt(process.env.PORT) : 8000
}, [], {
  requests: 0
})

// Data format
// <type Uint8> <Uint16> <Uint16> <Uint8>

const colors: string[] = Array.from({ length: 256 }, (_, i) => {
  let r = 0, g = 0, b = 0

  if (i % 3 === 0) {
    r = i
    g = Math.floor(i / 2)
    b = Math.floor(g / 2)
  } else if (i % 3 === 1) {
    g = i
    r = Math.floor(i / 2)
    b = Math.floor(r / 2)
  } else {
    b = i
    g = Math.floor(i / 2)
    r = Math.floor(g / 2)
  }

  return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`
})

const types = [
  'draw',
  'erase'
] as const

function toFormat(type: typeof types[number], x: number, y: number, color: string): Buffer {
  const uint8Color = colors.findIndex((c) => c === color)
  if (uint8Color === -1) throw 'Invalid Format'

  const uint8X = new Uint8Array([x & 0xFF, (x >> 8) & 0xFF])
  const uint8Y = new Uint8Array([y & 0xFF, (y >> 8) & 0xFF])

  const buffer = Buffer.from([
    types.findIndex((v) => v === type),
    ...uint8X,
    ...uint8Y,
    uint8Color
  ])

  return buffer
}

function fromFormat(buffer: Buffer): [type: typeof types[number], x: number, y: number] {
  const type = buffer[0]
  const x = buffer.readUInt16LE(1)
  const y = buffer.readUInt16LE(3)

  if (!types[type] || !x || !y) throw 'Invalid Format'

  return [types[type], x, y]
}

const drawChannel = new Channel<Buffer>()

const history: Buffer[] = []
drawChannel.listen((data) => {
  history.push(data)
  write?.write(Buffer.concat([data, seperator]))
})

const file = fs.readFileSync('../history.raw')
let startIndex = 0
let endIndex = file.indexOf(seperator)
while (endIndex !== -1) {
  const data = file.subarray(startIndex, endIndex)
  if (data.byteLength > 1) history.push(data)
  startIndex = endIndex + seperator.length
  endIndex = file.indexOf(seperator, startIndex)
}

const users: Record<number, string> = {}

server.path('/', (path) => path
  .static('../static', {
    stripHtmlEnding: true
  })
  .ws('/', (ws) => ws
    .context<{
      id: number
    }>()
    .onUpgrade((ctr) => {
      let id: number | null = null
      for (let i = 0; i < 255; i++) {
        const real = (i + number.generateCrypto(1, 1000)) % 255

        if (!users[real]) id = real
      }

      if (!id) return ctr.status(ctr.$status.CONFLICT).print('No more slots')

      users[id] = 'lol'
      ctr["@"].id = id
    })
    .onOpen((ctr) => {
      ctr.printChannel(drawChannel)
    })
    .onMessage(async(ctr) => {
      try {
        const [ type, x, y ] = fromFormat(ctr.rawMessageBytes())

        await drawChannel.send('binary', toFormat(type, x, y, colors[ctr["@"].id]))
      } catch { }
    })
    .onClose((ctr) => {
      delete users[ctr["@"].id]
    })
  )
  .http('GET', '/history', (http) => http
    .onRequest((ctr) => {
      return ctr.print(Buffer.concat(
        history.flatMap((value) => [value, seperator])
      ))
    })
  )
)

let requests = 0
server.http((ctr) => {
  console.log(`${ctr.type} ${ctr.url.method} Request made to ${ctr.url.href}`)

  ctr["@"].requests = ++requests
})

server.start()
  .then((port) => {
    console.log(`Server started on port ${port}`)
  })
  .catch(console.error)