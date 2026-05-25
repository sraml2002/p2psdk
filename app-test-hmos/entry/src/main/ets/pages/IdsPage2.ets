/**
 * IdsPage2 — One-click IDS P2P Chat.
 *
 * Single flow: init → registerIds → queryIds → connect → chat
 */
import { router } from '@kit.ArkUI'
import { hilog } from '@kit.PerformanceAnalysisKit'
import { pushService } from '@kit.PushKit'
import { deviceInfo } from '@kit.BasicServicesKit'
import { util } from '@kit.ArkTS'
import ppsdk from 'libppsdk.so'
import { loadAppConfig, AppConfig } from '../sdk/AppConfig'

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

interface ChatMessage {
  id: string
  text: string
  isSelf: boolean
  time: string
}

const HEARTBEAT_TIMEOUT_MS = 60000

@Entry
@Component
struct IdsPage2 {
  @State phase: string = 'idle' // idle | connecting | connected | failed
  @State statusText: string = ''
  @State errorMsg: string = ''
  @State chatMessages: ChatMessage[] = []
  @State inputText: string = ''
  @State logs: string[] = []
  @State lastHeartbeat: string = ''
  @State connectTimeMs: number = 0

  private heartbeatTimerId: number = -1
  private connectStartTime: number = 0
  private config: AppConfig = { idsUrl: '', natUrl: '', appId: '', userId: '' }

  aboutToAppear(): void {
    this.config = loadAppConfig(getContext(this))
    ppsdk.onStateChange((state: string): void => {
      this.appendLog(`ICE state: ${state}`)
      if (state === 'COMPLETED' || state === 'CONNECTED') {
        if (this.connectStartTime > 0) {
          this.connectTimeMs = Date.now() - this.connectStartTime
        }
        this.phase = 'connected'
        this.statusText = `P2P 连接已建立 (${this.connectTimeMs}ms)`
        this.resetHeartbeatTimer()
      } else if (state === 'FAILED') {
        this.setError('ICE 连接失败')
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

  private setError(msg: string): void {
    this.phase = 'failed'
    this.errorMsg = msg
    this.statusText = ''
  }

  private resetHeartbeatTimer(): void {
    if (this.heartbeatTimerId !== -1) {
      clearTimeout(this.heartbeatTimerId)
    }
    this.lastHeartbeat = new Date().toLocaleTimeString()
    this.heartbeatTimerId = setTimeout(() => {
      this.appendLog('Heartbeat timeout — connection lost')
      this.setError('心跳超时，连接断开')
      this.heartbeatTimerId = -1
    }, HEARTBEAT_TIMEOUT_MS)
  }

  // ── One-click connect ─────────────────────────────────────────────

  private async startOneClickConnect(): Promise<void> {
    this.phase = 'connecting'
    this.errorMsg = ''
    this.statusText = '正在初始化...'
    this.logs = []
    this.connectStartTime = Date.now()

    // Step 1: init
    const initCode: number = ppsdk.init(JSON.stringify({
      idsUrl: this.config.idsUrl,
      natUrl: this.config.natUrl,
    }))
    if (initCode !== 0) {
      this.setError('init 失败')
      return
    }
    this.appendLog('init OK')

    // Step 2: registerIds
    this.statusText = '正在注册 IDS...'
    let pushToken: string = ''
    try {
      pushToken = await pushService.getToken()
    } catch (_e) {
      this.appendLog('PushToken unavailable, using empty token')
    }

    const odid: string = deviceInfo.ODID || this.config.userId
    try {
      const resp: IdsResponse = ppsdk.registerIds(this.config.appId, this.config.userId, odid, pushToken)
      if (resp.error !== undefined && resp.error.length > 0) {
        this.setError(`IDS 注册失败: ${resp.error}`)
        return
      }
    } catch (e) {
      this.setError(`IDS 注册异常: ${JSON.stringify(e)}`)
      return
    }
    this.appendLog('registerIds OK')

    // Step 3: queryIds
    this.statusText = '正在查询服务...'
    let signalingUrl: string = ''
    try {
      const resp: IdsResponse = ppsdk.queryIds(this.config.appId, this.config.userId)
      if (resp.data === undefined || resp.data.length === 0) {
        this.setError('IDS 查询无结果')
        return
      }
      let found: boolean = false
      for (let i = 0; i < resp.data.length; i++) {
        const record: IdsRecord = resp.data[i]
        if (record.type === 'service' && record.token.length > 0) {
          signalingUrl = record.token
          found = true
          break
        }
      }
      if (!found) {
        this.setError('未找到 service 记录')
        return
      }
    } catch (e) {
      this.setError(`IDS 查询异常: ${JSON.stringify(e)}`)
      return
    }
    this.appendLog(`queryIds OK, signalingUrl: ${signalingUrl}`)

    // Step 4: connect (one-stop: generateToken + gatherCandidates + iceSdpNegotiate)
    this.statusText = '正在建立 P2P 连接...'
    const code: number = ppsdk.connect(signalingUrl, odid)
    if (code !== 0) {
      this.setError('connect 调用失败')
      return
    }
    this.appendLog('connect initiated, waiting for ICE...')
  }

  // ── Chat ──────────────────────────────────────────────────────────

  private handleChatData(data: ArrayBuffer): void {
    const frame: ParsedFrame = ppsdk.parseFrame(data)
    if (frame.type === 0) {
      return
    }
    if (frame.type === 1) {
      this.resetHeartbeatTimer()
      const reply: ArrayBuffer = ppsdk.encodeHeartbeatReply()
      ppsdk.send(reply)
    } else if (frame.type === 2) {
      const payloadBytes = new Uint8Array(frame.payload)
      const text: string = new util.TextDecoder().decodeToString(payloadBytes)
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
    if (this.phase !== 'connected' || this.inputText.length === 0) return

    const text: string = this.inputText
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
        Text('P2P Chat')
          .fontSize(18)
          .fontWeight(FontWeight.Bold)
          .margin({ left: 12 })
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 12, bottom: 8 })
      .alignItems(VerticalAlign.Center)

      // Connecting phase: show status + error
      if (this.phase === 'idle') {
        Column({ space: 16 }) {
          Text('一键 P2P 连接')
            .fontSize(18)
            .fontWeight(FontWeight.Bold)
            .fontColor('#333333')

          Text('自动完成: 初始化 → 注册 IDS → 查询服务 → 建立 P2P 连接')
            .fontSize(14)
            .fontColor('#666666')
            .textAlign(TextAlign.Center)

          Button('开始连接')
            .width('80%')
            .fontSize(18)
            .fontColor('#FFFFFF')
            .backgroundColor('#7B1FA2')
            .borderRadius(24)
            .onClick(() => {
              this.startOneClickConnect()
            })
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
        .alignItems(HorizontalAlign.Center)
        .padding({ left: 24, right: 24 })
      }

      if (this.phase === 'connecting') {
        Column({ space: 16 }) {
          LoadingProgress()
            .width(48)
            .height(48)
            .color('#7B1FA2')

          Text(this.statusText)
            .fontSize(16)
            .fontColor('#333333')
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
        .alignItems(HorizontalAlign.Center)
      }

      if (this.phase === 'failed') {
        Column({ space: 16 }) {
          Text('连接失败')
            .fontSize(18)
            .fontWeight(FontWeight.Bold)
            .fontColor('#E53935')

          Text(this.errorMsg)
            .fontSize(14)
            .fontColor('#666666')
            .textAlign(TextAlign.Center)
            .wordBreak(WordBreak.BREAK_ALL)
            .width('80%')

          Button('重新连接')
            .width('80%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#7B1FA2')
            .borderRadius(24)
            .onClick(() => {
              this.startOneClickConnect()
            })
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
        .alignItems(HorizontalAlign.Center)
        .padding({ left: 24, right: 24 })
      }

      // Connected: show chat UI
      if (this.phase === 'connected') {
        // Connection info bar
        Row() {
          Text('P2P 已连接')
            .fontSize(14)
            .fontWeight(FontWeight.Bold)
            .fontColor('#4CAF50')
          Text(`  建链耗时: ${this.connectTimeMs}ms`)
            .fontSize(12)
            .fontColor('#333333')
          if (this.lastHeartbeat.length > 0) {
            Text(`  HB: ${this.lastHeartbeat}`)
              .fontSize(12)
              .fontColor('#999999')
          }
        }
        .width('90%')
        .padding({ top: 4, bottom: 4 })

        // Chat messages
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
          TextInput({ text: this.inputText, placeholder: '输入消息...' })
            .layoutWeight(1)
            .fontSize(14)
            .onChange((value: string) => {
              this.inputText = value
            })
            .onSubmit(() => {
              this.sendMessage()
            })

          Button('发送')
            .fontSize(14)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(16)
            .enabled(this.inputText.length > 0)
            .onClick(() => {
              this.sendMessage()
            })
        }
        .width('90%')
        .padding({ top: 8, bottom: 8 })
      }

      // Log area
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
