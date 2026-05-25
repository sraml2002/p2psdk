import { pushService } from '@kit.PushKit'
import { router } from '@kit.ArkUI'
import { hilog } from '@kit.PerformanceAnalysisKit'
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

@Entry
@Component
struct Index {
  @State hours: number = 0
  @State minutes: number = 0
  @State seconds: number = 0
  @State isAnalog: boolean = true
  @State isGettingToken: boolean = false
  @State isGatheringCandidates: boolean = false
  @State candidates: string[] = []

  private timerId: number = -1
  private hourNums: number[] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
  private hourTicks: number[] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
  private minuteTicks: number[] = []

  aboutToAppear(): void {
    hilog.info(0x0000, 'testTag', '[V2] Index page aboutToAppear')
    for (let i = 0; i < 60; i++) {
      if (i % 5 !== 0) {
        this.minuteTicks.push(i)
      }
    }
    this.updateTime()
    this.startTimer()
    const config: AppConfig = loadAppConfig(getContext(this))
    ppsdk.init(JSON.stringify({
      idsUrl: config.idsUrl,
      natUrl: config.natUrl,
    }))
  }

  aboutToDisappear(): void {
    hilog.info(0x0000, 'testTag', '[V2] Index page aboutToDisappear')
    if (this.timerId !== -1) {
      clearTimeout(this.timerId)
      this.timerId = -1
    }
  }

  startTimer(): void {
    this.timerId = setTimeout(() => {
      this.updateTime()
      this.startTimer()
    }, 1000)
  }

  updateTime(): void {
    const now = new Date()
    this.hours = now.getHours()
    this.minutes = now.getMinutes()
    this.seconds = now.getSeconds()
  }

  padZero(n: number): string {
    return n < 10 ? `0${n}` : `${n}`
  }

  getPushToken(): void {
    this.isGettingToken = true
    pushService.getToken().then((token: string) => {
      router.pushUrl({ url: 'pages/PushTokenPage', params: { token: token } })
    }).catch((err: Error) => {
      hilog.error(0x0000, 'testTag', '[V2] getPushToken failed: %{public}s', JSON.stringify(err))
    }).finally(() => {
      this.isGettingToken = false
    })
  }

  gatherAllCandidates(): void {
    if (this.isGatheringCandidates) {
      return
    }
    this.isGatheringCandidates = true
    this.candidates = ['正在获取候选地址...']

    const token: string = ppsdk.generateToken()
    if (token.length === 0) {
      this.candidates = ['Token 获取失败']
      this.isGatheringCandidates = false
      return
    }

    try {
      const info: CandidateInfo = ppsdk.gatherCandidates(token)
      this.candidates = info.candidateLines.length > 0 ? info.candidateLines : ['未获取到候选地址']
    } catch (e) {
      this.candidates = [`获取失败: ${JSON.stringify(e)}`]
    }
    this.isGatheringCandidates = false
  }

  build() {
    Column() {
      // 时钟区域
      Column() {
        if (this.isAnalog) {
          Stack() {
            Circle({ width: 280, height: 280 })
              .fill('#FFFFFF')
              .stroke('#333333')
              .strokeWidth(3)

            ForEach(this.minuteTicks, (i: number) => {
              Line()
                .width(280)
                .height(280)
                .startPoint([140, 6])
                .endPoint([140, 14])
                .stroke('#CCCCCC')
                .strokeWidth(1)
                .rotate({ angle: i * 6, centerX: 140, centerY: 140 })
            }, (i: number) => `mtick_${i}`)

            ForEach(this.hourTicks, (i: number) => {
              Line()
                .width(280)
                .height(280)
                .startPoint([140, 6])
                .endPoint([140, i % 3 === 0 ? 26 : 18])
                .stroke(i % 3 === 0 ? '#333333' : '#999999')
                .strokeWidth(i % 3 === 0 ? 3 : 1.5)
                .strokeLineCap(LineCapStyle.Round)
                .rotate({ angle: i * 30, centerX: 140, centerY: 140 })
            }, (i: number) => `htick_${i}`)

            ForEach(this.hourNums, (num: number) => {
              Text(`${num}`)
                .fontSize(16)
                .fontColor('#333333')
                .width(24)
                .height(24)
                .textAlign(TextAlign.Center)
                .position({
                  x: 128 + 98 * Math.sin(num * 30 * Math.PI / 180),
                  y: 128 - 98 * Math.cos(num * 30 * Math.PI / 180)
                })
            }, (num: number) => `num_${num}`)

            Line()
              .width(280)
              .height(280)
              .startPoint([140, 155])
              .endPoint([140, 75])
              .stroke('#333333')
              .strokeWidth(6)
              .strokeLineCap(LineCapStyle.Round)
              .rotate({
                angle: ((this.hours % 12) + this.minutes / 60 + this.seconds / 3600) * 30,
                centerX: 140, centerY: 140
              })

            Line()
              .width(280)
              .height(280)
              .startPoint([140, 155])
              .endPoint([140, 45])
              .stroke('#555555')
              .strokeWidth(4)
              .strokeLineCap(LineCapStyle.Round)
              .rotate({
                angle: (this.minutes + this.seconds / 60) * 6,
                centerX: 140, centerY: 140
              })

            Line()
              .width(280)
              .height(280)
              .startPoint([140, 160])
              .endPoint([140, 30])
              .stroke('#E53935')
              .strokeWidth(1.5)
              .strokeLineCap(LineCapStyle.Round)
              .rotate({
                angle: this.seconds * 6,
                centerX: 140, centerY: 140
              })

            Circle({ width: 10, height: 10 })
              .fill('#E53935')
          }
          .width(280)
          .height(280)
        } else {
          Text(`${this.padZero(this.hours)}:${this.padZero(this.minutes)}:${this.padZero(this.seconds)}`)
            .fontSize(72)
            .fontWeight(FontWeight.Bold)
            .fontColor('#333333')
            .letterSpacing(4)
        }
      }
      .layoutWeight(1)
      .justifyContent(FlexAlign.Center)
      .alignItems(HorizontalAlign.Center)

      // 按钮区域
      Scroll() {
        Column({ space: 16 }) {
          Button(this.isAnalog ? '切换到数字时钟' : '切换到表盘时钟')
            .width('80%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(20)
            .onClick(() => {
              this.isAnalog = !this.isAnalog
            })

          Button(this.isGettingToken ? '获取中...' : '获取PushToken')
            .width('80%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(20)
            .enabled(!this.isGettingToken)
            .onClick(() => {
              this.getPushToken()
            })

          Button(this.isGatheringCandidates ? '获取中...' : '获取候选地址')
            .width('80%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(20)
            .enabled(!this.isGatheringCandidates)
            .onClick(() => {
              this.gatherAllCandidates()
            })

          if (this.candidates.length > 0) {
            Column({ space: 4 }) {
              Text('ICE 候选地址')
                .fontSize(13)
                .fontWeight(FontWeight.Bold)
                .fontColor('#333333')
                .width('80%')
              Text(this.candidates.join('\n'))
                .fontSize(12)
                .fontColor('#333333')
                .width('80%')
                .wordBreak(WordBreak.BREAK_ALL)
            }
            .width('100%')
            .alignItems(HorizontalAlign.Center)
            .margin({ top: 8 })
          }

          Button('P2P建链调试')
            .width('80%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#7B1FA2')
            .borderRadius(20)
            .onClick(() => {
              router.pushUrl({ url: 'pages/IdsPage' })
            })

          Button('P2P Chat')
            .width('80%')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#E65100')
            .borderRadius(20)
            .onClick(() => {
              router.pushUrl({ url: 'pages/IdsPage2' })
            })
        }
        .width('100%')
        .padding({ bottom: 48 })
      }
      .scrollable(ScrollDirection.Vertical)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#F5F5F5')
  }
}
