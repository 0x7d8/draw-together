import { Channel, Server } from "rjweb-server"
import { Runtime } from "@rjweb/runtime-node"
import { number } from "@rjweb/utils"
import * as fs from "fs"
import path from "path"

const seperator = Buffer.from([0x69, 0x62, 0x64, 0x69]),
  options = {
    nosave: process.argv.includes('--nosave')
  }

if (!fs.existsSync(path.join(__dirname, '../history.raw')) && !options.nosave) fs.writeFileSync(path.join(__dirname, '../history.raw'), Buffer.concat([Buffer.from([0x01]), seperator]))
const write = !options.nosave ? fs.createWriteStream(path.join(__dirname, '../history.raw'), {
  flags: 'a'
}) : null

const server = new Server(Runtime, {
  port: process.env.PORT ? parseInt(process.env.PORT) : 8000,
  proxy: {
    enabled: true
  }
}, [], {
  requests: 0
})

// Data format
// <type Uint8> <Uint16> <Uint16> <Uint8> <Uint8>

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

function toFormat(type: typeof types[number], x: number, y: number, height: number, color: string): Buffer {
  const uint8Color = colors.findIndex((c) => c === color)
  if (uint8Color === -1) throw 'Invalid Format'

  const uint8X = new Uint8Array([x & 0xFF, (x >> 8) & 0xFF])
  const uint8Y = new Uint8Array([y & 0xFF, (y >> 8) & 0xFF])

  const buffer = Buffer.from([
    types.findIndex((v) => v === type),
    ...uint8X,
    ...uint8Y,
    uint8Color,
    height
  ])

  return buffer
}

function fromFormat(buffer: Buffer): [type: typeof types[number], x: number, y: number, height: number] {
  const type = buffer[0]
  const x = buffer.readUInt16LE(1)
  const y = buffer.readUInt16LE(3)
  const height = buffer[5] || 5

  if (!types[type] || !x || !y) throw 'Invalid Format'

  return [types[type], x, y, height]
}

const drawChannel = new Channel<Buffer>()

const history: Buffer[] = []
drawChannel.listen((data) => {
  if (options.nosave) history.push(data, seperator)

  write?.write(Buffer.concat([data, seperator]))
})

if (fs.existsSync(path.join(__dirname, '../history.raw')) && options.nosave) {
  const now = performance.now()
  console.log('loading history...')

  const buffer = fs.readFileSync(path.join(__dirname, '../history.raw'))

  let startIndex = 0
  let endIndex = buffer.indexOf(seperator)
  while (endIndex !== -1) {
    const data = buffer.subarray(startIndex, endIndex)
    if (data.byteLength > 1) history.push(data, seperator)
    startIndex = endIndex + seperator.length
    endIndex = buffer.indexOf(seperator, startIndex)
  }

  console.log(`loaded history in ${(performance.now() - now).toFixed(2)}ms with ${history.length} entries`)
}

const users = new Map<number, string>()

server.path('/', (p) => p
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

        if (!users.has(real)) id = real
      }

      if (!id) return ctr.status(ctr.$status.CONFLICT).print('No more slots')

      users.set(id, 'lol')
      ctr["@"].id = id
    })
    .onOpen((ctr) => {
      ctr.printChannel(drawChannel)
    })
    .onMessage(async(ctr) => {
      const bytes = ctr.rawMessageBytes()
      if (bytes.byteLength !== 6) return

      try {
        const [ type, x, y, height ] = fromFormat(bytes)

        await drawChannel.send('binary', toFormat(type, x, y, height, colors[ctr["@"].id]))
      } catch { }
    })
    .onClose((ctr) => {
      users.delete(ctr["@"].id)
    })
  )
  .http('GET', '/history', (http) => http
    .onRequest((ctr) => {
      ctr.headers.set('content-type', 'robert/history-raw')

      if (!options.nosave) return ctr.printFile(path.join(__dirname, '../history.raw'), { compress: true, addTypes: false })
      else return ctr.print(Buffer.concat(history))
    })
  )
)

let requests = 0
server.http((ctr) => {
  console.log(`${ctr.type.toUpperCase()} ${ctr.url.method} Request made to ${ctr.url.href} (${ctr.client.ip.usual()})`)

  ctr["@"].requests = ++requests
})

server.start()
  .then((port) => {
    console.log(`server started on port ${port}`)
  })
  .catch(console.error)