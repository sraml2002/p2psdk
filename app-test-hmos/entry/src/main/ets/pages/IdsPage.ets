/**
 * IdsPage — IDS + HSService P2P Chat page.
 *
 * Four-step flow:
 *   Step 1: Register to IDS
 *   Step 2: Gather host candidates
 *   Step 3: Query IDS for HSService
 *   Step 4: Send SDP offer, negotiate ICE, start chat
 */
import { router } from '@kit.ArkUI'
import { hilog } from '@kit.PerformanceAnalysisKit'
import { pushService } from '@kit.PushKit'
import { deviceInfo } from '@kit.BasicServicesKit'
import { util } from '@kit.ArkTS'
import ppsdk from 'libppsdk.so'
import { loadAppConfig, AppConfig } from '../sdk/AppConfig'

interface CandidateInfo {
  candidateLines: string[]
  localAddresses: string[]
  stunExternalIp: string
  stunExternalPort: string
  turnRelayIp: string
  turnRelayPort: string
}

interface IdsResponse {
  code: number
  message: string
  error: string | undefined
  data: IdsRecord[] | undefined
}

interface IdsRecord {
  appId: string
  userId: string
  type: string
  odid: string
  token: string
}

interface ParsedFrame {
  type: number
  payload: ArrayBuffer
}

const TAG = 'IdsPage'
const HEARTBEAT_TIMEOUT_MS = 60000

interface ChatMessage {
  id: string
  text: string
  isSelf: boolean
  time: string
}

@Entry
@Component
struct IdsPage {
  @State step: number = 1
  @State gatheringStatus: string = ''
  @State candidates: string[] = []
  @State idsStatus: string = ''
  @State signalingUrl: string = ''
  @State iceStatus: string = 'NEW'
  @State connected: boolean = false
  @State disconnected: boolean = false
  @State chatMessages: ChatMessage[] = []
  @State inputText: string = ''
  @State logs: string[] = []
  @State lastHeartbeat: string = ''
  @State isGathering: boolean = false
  @State isQuerying: boolean = false
  @State isConnecting: boolean = false
  @State isRegistering: boolean = false
  @State registerStatus: string = ''

  private heartbeatTimerId: number = -1
  private config: AppConfig = { idsUrl: '', natUrl: '', appId: '', userId: '' }

  aboutToAppear(): void {
    this.config = loadAppConfig(getContext(this))
    ppsdk.init(JSON.stringify({
      idsUrl: this.config.idsUrl,
      natUrl: this.config.natUrl,
    }))

    ppsdk.onStateChange((state: string): void => {
      this.iceStatus = state
      this.appendLog(`ICE state: ${state}`)
      if (state === 'COMPLETED' || state === 'CONNECTED') {
        this.connected = true
        this.isConnecting = false
        this.resetHeartbeatTimer()
        this.appendLog('P2P channel established!')
      } else if (state === 'FAILED') {
        this.connected = false
        this.isConnecting = false
      }
    })
    ppsdk.onLog((msg: string): void => {
      this.appendLog(msg)
    })
    ppsdk.onDataReceived((data: ArrayBuffer): void => {
      this.handleChatData(data)
    })
  }

  aboutToDisappear(): void {
    this.cleanup()
  }

  private cleanup(): void {
    if (this.heartbeatTimerId !== -1) {
      clearTimeout(this.heartbeatTimerId)
      this.heartbeatTimerId = -1
    }
    ppsdk.close()
  }

  private appendLog(msg: string): void {
    const ts = new Date().toLocaleTimeString()
    this.logs = [...this.logs, `[${ts}] ${msg}`]
    if (this.logs.length > 200) {
      this.logs = this.logs.slice(-200)
    }
  }

  private resetHeartbeatTimer(): void {
    if (this.heartbeatTimerId !== -1) {
      clearTimeout(this.heartbeatTimerId)
    }
    this.lastHeartbeat = new Date().toLocaleTimeString()
    this.heartbeatTimerId = setTimeout(() => {
      this.appendLog('Heartbeat timeout — connection lost')
      this.disconnected = true
      this.connected = false
      this.heartbeatTimerId = -1
    }, HEARTBEAT_TIMEOUT_MS)
  }

  // ── Step 1: Register to IDS ─────────────────────────────────────

  doRegisterIds(): void {
    this.isRegistering = true
    this.registerStatus = 'Registering...'
    this.appendLog('Step 1: Registering to IDS...')

    let pushToken: string = ''
    pushService.getToken().then((token: string) => {
      pushToken = token
    }).catch(() => {
      this.appendLog('PushToken unavailable, using empty token')
    }).finally(() => {
      try {
        const odid: string = deviceInfo.ODID || this.config.userId
        const resp: IdsResponse = ppsdk.registerIds(this.config.appId, this.config.userId, odid, pushToken)
        if (resp.error !== undefined && resp.error.length > 0) {
          this.registerStatus = 'Failed'
          this.appendLog(`IDS registration failed: ${resp.error}`)
        } else {
          this.registerStatus = 'Registered successfully'
          this.appendLog('IDS registration succeeded')
        }
      } catch (e) {
        this.registerStatus = 'Failed'
        this.appendLog(`IDS registration error: ${JSON.stringify(e)}`)
      }
      this.isRegistering = false
    })
  }

  // ── Step 2: Gather candidates ──────────────────────────────────────

  doGatherCandidates(): void {
    this.isGathering = true
    this.gatheringStatus = 'Gathering...'
    this.candidates = []
    this.appendLog('Step 2: Gathering candidates')

    const token: string = ppsdk.generateToken()
    if (token.length === 0) {
      this.gatheringStatus = 'Failed: token not available'
      this.isGathering = false
      return
    }

    try {
      const info: CandidateInfo = ppsdk.gatherCandidates(token)
      this.candidates = info.candidateLines
      this.gatheringStatus = `Done (${info.candidateLines.length} candidates)`
    } catch (e) {
      this.gatheringStatus = 'Failed'
      this.appendLog(`Gather error: ${JSON.stringify(e)}`)
    }
    this.isGathering = false
  }

  // ── Step 3: Query IDS ─────────────────────────────────────────────

  doQueryIds(): void {
    this.isQuerying = true
    this.idsStatus = 'Querying...'
    this.appendLog(`Step 3: Query IDS ${this.config.idsUrl}/${this.config.appId}/${this.config.userId}`)

    try {
      const resp: IdsResponse = ppsdk.queryIds(this.config.appId, this.config.userId)
      if (resp.data !== undefined && resp.data.length > 0) {
        let found: boolean = false
        for (let i = 0; i < resp.data.length; i++) {
          const record: IdsRecord = resp.data[i]
          this.appendLog(`IDS result[${i}]: type=${record.type}, token=${record.token}`)
          if (record.type === 'service' && record.token.length > 0) {
            this.signalingUrl = record.token
            this.idsStatus = `Found: service @ ${record.token}`
            found = true
            break
          }
        }
        if (!found) {
          this.idsStatus = 'Failed: no service record found'
        }
      } else {
        this.idsStatus = 'Failed: no service record found'
      }
    } catch (e) {
      this.idsStatus = 'Failed'
      this.appendLog(`Query error: ${JSON.stringify(e)}`)
    }
    this.isQuerying = false
  }

  // ── Step 4: Initiate P2P connection ───────────────────────────────

  doConnect(): void {
    this.isConnecting = true
    this.iceStatus = 'CONNECTING'
    this.connected = false
    this.disconnected = false
    this.appendLog('Step 4: Initiating P2P connection')

    const code: number = ppsdk.iceSdpNegotiate(this.signalingUrl, deviceInfo.ODID || this.config.userId)
    if (code !== 0) {
      this.iceStatus = 'FAILED'
      this.appendLog('Connect failed')
      this.isConnecting = false
    }
  }

  // ── Chat data handling ────────────────────────────────────────────

  private handleChatData(data: ArrayBuffer): void {
    const frame: ParsedFrame = ppsdk.parseFrame(data)
    if (frame.type === 0) {
      this.appendLog('Invalid P2P frame received')
      return
    }

    if (frame.type === 1) {
      this.appendLog('Heartbeat received, replying')
      this.resetHeartbeatTimer()
      const reply: ArrayBuffer = ppsdk.encodeHeartbeatReply()
      ppsdk.send(reply)
    } else if (frame.type === 2) {
      const payloadBytes = new Uint8Array(frame.payload)
      const text = new util.TextDecoder().decodeToString(payloadBytes)
      this.appendLog(`Data received: ${text}`)
      const msg: ChatMessage = {
        id: `${Date.now()}`,
        text: text,
        isSelf: false,
        time: new Date().toLocaleTimeString(),
      }
      this.chatMessages = [...this.chatMessages, msg]
      this.resetHeartbeatTimer()
    }
  }

  private sendMessage(): void {
    if (!this.connected || this.inputText.length === 0) return

    const text = this.inputText
    this.inputText = ''

    const frame: ArrayBuffer = ppsdk.encodeDataFrame(text)
    ppsdk.send(frame)

    const msg: ChatMessage = {
      id: `${Date.now()}`,
      text: text,
      isSelf: true,
      time: new Date().toLocaleTimeString(),
    }
    this.chatMessages = [...this.chatMessages, msg]
    this.appendLog(`Sent: ${text}`)
  }

  // ── Build UI ──────────────────────────────────────────────────────

  build() {
    Column() {
      // Title bar
      Row() {
        Button('< Back')
          .fontSize(14)
          .fontColor('#FFFFFF')
          .backgroundColor('#999999')
          .borderRadius(12)
          .onClick(() => {
            router.back()
          })
        Text('IDS P2P Chat')
          .fontSize(18)
          .fontWeight(FontWeight.Bold)
          .margin({ left: 12 })
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 12, bottom: 8 })
      .alignItems(VerticalAlign.Center)

      // Step indicators
      Row({ space: 8 }) {
        Text('1')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#FFFFFF')
          .width(28)
          .height(28)
          .textAlign(TextAlign.Center)
          .backgroundColor(this.step >= 1 ? '#007DFF' : '#CCCCCC')
          .borderRadius(14)
        Text('2')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#FFFFFF')
          .width(28)
          .height(28)
          .textAlign(TextAlign.Center)
          .backgroundColor(this.step >= 2 ? '#007DFF' : '#CCCCCC')
          .borderRadius(14)
        Text('3')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#FFFFFF')
          .width(28)
          .height(28)
          .textAlign(TextAlign.Center)
          .backgroundColor(this.step >= 3 ? '#007DFF' : '#CCCCCC')
          .borderRadius(14)
        Text('4')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#FFFFFF')
          .width(28)
          .height(28)
          .textAlign(TextAlign.Center)
          .backgroundColor(this.step >= 4 ? '#007DFF' : '#CCCCCC')
          .borderRadius(14)
      }
      .width('100%')
      .padding({ left: 16 })
      .justifyContent(FlexAlign.Start)

      // Step 1: Register to IDS
      if (this.step === 1) {
        Column({ space: 12 }) {
          Text('Step 1: Register to IDS')
            .fontSize(16)
            .fontWeight(FontWeight.Bold)
            .width('90%')

          Button(this.isRegistering ? 'Registering...' : 'Register to IDS')
            .width('90%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#FF6D00')
            .borderRadius(20)
            .enabled(!this.isRegistering)
            .onClick(() => {
              this.doRegisterIds()
            })

          Text(this.registerStatus)
            .fontSize(14)
            .fontColor('#666666')
            .width('90%')
            .wordBreak(WordBreak.BREAK_ALL)

          Button('Next: Gather Candidates >')
            .width('90%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#2E7D32')
            .borderRadius(20)
            .onClick(() => {
              this.step = 2
            })
        }
        .width('100%')
        .padding({ left: 16, right: 16, top: 12 })
        .alignItems(HorizontalAlign.Center)
      }

      // Step 2: Gather candidates
      if (this.step === 2) {
        Column({ space: 12 }) {
          Text('Step 2: Gather Candidates')
            .fontSize(16)
            .fontWeight(FontWeight.Bold)
            .width('90%')

          Button(this.isGathering ? 'Gathering...' : 'Gather Candidates')
            .width('90%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(20)
            .enabled(!this.isGathering)
            .onClick(() => {
              this.doGatherCandidates()
            })

          Text(this.gatheringStatus)
            .fontSize(14)
            .fontColor('#666666')
            .width('90%')

          if (this.candidates.length > 0) {
            ForEach(this.candidates, (line: string) => {
              Text(line)
                .fontSize(12)
                .fontColor('#333333')
                .width('90%')
                .wordBreak(WordBreak.BREAK_ALL)
            }, (line: string, index: number) => `cand_${index}`)
          }

          if (this.candidates.length > 0) {
            Button('Next: Query IDS >')
              .width('90%')
              .fontSize(16)
              .fontColor('#FFFFFF')
              .backgroundColor('#2E7D32')
              .borderRadius(20)
              .onClick(() => {
                this.step = 3
              })
          }

          Button('< Back')
            .width('90%')
            .fontSize(14)
            .fontColor('#FFFFFF')
            .backgroundColor('#999999')
            .borderRadius(16)
            .onClick(() => {
              this.step = 1
            })
        }
        .width('100%')
        .padding({ left: 16, right: 16, top: 12 })
        .alignItems(HorizontalAlign.Center)
      }

      // Step 3: Query IDS
      if (this.step === 3) {
        Column({ space: 12 }) {
          Text('Step 3: Query IDS Service')
            .fontSize(16)
            .fontWeight(FontWeight.Bold)
            .width('90%')

          Button(this.isQuerying ? 'Querying...' : 'Query IDS')
            .width('90%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(20)
            .enabled(!this.isQuerying)
            .onClick(() => {
              this.doQueryIds()
            })

          Text(this.idsStatus)
            .fontSize(14)
            .fontColor('#666666')
            .width('90%')
            .wordBreak(WordBreak.BREAK_ALL)

          if (this.signalingUrl.length > 0) {
            Button('Next: Connect P2P >')
              .width('90%')
              .fontSize(16)
              .fontColor('#FFFFFF')
              .backgroundColor('#2E7D32')
              .borderRadius(20)
              .onClick(() => {
                this.step = 4
              })
          }

          Button('< Back')
            .width('90%')
            .fontSize(14)
            .fontColor('#FFFFFF')
            .backgroundColor('#999999')
            .borderRadius(16)
            .onClick(() => {
              this.step = 2
            })
        }
        .width('100%')
        .padding({ left: 16, right: 16, top: 12 })
        .alignItems(HorizontalAlign.Center)
      }

      // Step 4: P2P Connection
      if (this.step === 4) {
        Column() {
          // Connection status
          Row({ space: 12 }) {
            Text(`ICE: ${this.iceStatus}`)
              .fontSize(14)
              .fontWeight(FontWeight.Bold)
              .fontColor(
                this.connected ? '#4CAF50' :
                this.disconnected ? '#E53935' :
                '#333333'
              )
            if (this.lastHeartbeat.length > 0) {
              Text(`HB: ${this.lastHeartbeat}`)
                .fontSize(12)
                .fontColor('#999999')
            }
          }
          .width('90%')

          // Connect button
          if (!this.connected && !this.disconnected) {
            Button(this.isConnecting ? 'Connecting...' : 'Start P2P Connection')
              .width('90%')
              .fontSize(16)
              .fontColor('#FFFFFF')
              .backgroundColor('#007DFF')
              .borderRadius(20)
              .enabled(!this.isConnecting)
              .onClick(() => {
                this.doConnect()
              })
          }

          // Chat area
          if (this.connected || this.disconnected || this.chatMessages.length > 0) {
            Text(this.disconnected ? 'Disconnected' : 'Chat')
              .fontSize(14)
              .fontWeight(FontWeight.Bold)
              .fontColor(this.disconnected ? '#E53935' : '#333333')
              .width('90%')
              .margin({ top: 8 })

            // Message list
            List() {
              ForEach(this.chatMessages, (msg: ChatMessage) => {
                ListItem() {
                  Row() {
                    if (msg.isSelf) {
                      Blank().layoutWeight(1)
                      Text(msg.text)
                        .fontSize(14)
                        .fontColor('#FFFFFF')
                        .backgroundColor('#007DFF')
                        .borderRadius(12)
                        .padding({ left: 12, right: 12, top: 8, bottom: 8 })
                        .constraintSize({ maxWidth: '70%', minWidth: 40 })
                    } else {
                      Text(msg.text)
                        .fontSize(14)
                        .fontColor('#333333')
                        .backgroundColor('#FFFFFF')
                        .borderRadius(12)
                        .padding({ left: 12, right: 12, top: 8, bottom: 8 })
                        .constraintSize({ maxWidth: '70%', minWidth: 40 })
                      Blank().layoutWeight(1)
                    }
                  }
                  .width('100%')
                  .justifyContent(FlexAlign.Start)
                }
              }, (msg: ChatMessage) => msg.id)
            }
            .width('90%')
            .layoutWeight(1)

            // Input area
            Row({ space: 8 }) {
              TextInput({ text: this.inputText, placeholder: 'Type message...' })
                .layoutWeight(1)
                .fontSize(14)
                .onChange((value: string) => {
                  this.inputText = value
                })
                .onSubmit(() => {
                  this.sendMessage()
                })

              Button('Send')
                .fontSize(14)
                .fontColor('#FFFFFF')
                .backgroundColor('#007DFF')
                .borderRadius(16)
                .enabled(this.connected && this.inputText.length > 0)
                .onClick(() => {
                  this.sendMessage()
                })
            }
            .width('90%')
            .padding({ top: 8, bottom: 8 })
          }

          Button('< Back')
            .width('90%')
            .fontSize(14)
            .fontColor('#FFFFFF')
            .backgroundColor('#999999')
            .borderRadius(16)
            .onClick(() => {
              this.step = 3
            })
        }
        .width('100%')
        .layoutWeight(1)
        .padding({ left: 16, right: 16, top: 8 })
        .alignItems(HorizontalAlign.Center)
      }

      // Log area (collapsible at bottom)
      if (this.logs.length > 0) {
        Text('Logs')
          .fontSize(12)
          .fontWeight(FontWeight.Bold)
          .fontColor('#999999')
          .width('100%')
          .padding({ left: 16 })

        Scroll() {
          Column() {
            ForEach(this.logs, (line: string) => {
              Text(line)
                .fontSize(10)
                .fontColor('#777777')
                .width('90%')
                .wordBreak(WordBreak.BREAK_ALL)
            }, (line: string, index: number) => `log_${index}`)
          }
        }
        .width('100%')
        .height(100)
        .backgroundColor('#F0F0F0')
        .padding({ left: 16, right: 16 })
      }
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#F5F5F5')
  }
}
