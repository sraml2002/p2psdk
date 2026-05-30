import { pasteboard } from '@kit.BasicServicesKit'

@Entry
@Component
struct PushTokenPage {
  @State token: string = ''
  @State copyResult: string = ''
  @State hasError: boolean = false

  aboutToAppear(): void {
    const params = this.getUIContext().getRouter().getParams() as Record<string, string>
    if (params && params['token']) {
      this.token = params['token']
    } else {
      this.hasError = true
    }
  }

  copyToken(): void {
    try {
      const pasteData = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, this.token)
      const systemPasteboard = pasteboard.getSystemPasteboard()
      systemPasteboard.setDataSync(pasteData)
      this.copyResult = '已复制到剪贴板'
      setTimeout(() => {
        this.copyResult = ''
      }, 2000)
    } catch (e) {
      this.copyResult = '复制失败'
    }
  }

  build() {
    Column() {
      Text('PushToken')
        .fontSize(24)
        .fontWeight(FontWeight.Bold)
        .fontColor('#333333')
        .margin({ top: 60, bottom: 24 })

      if (this.hasError) {
        Text('获取Token失败，请返回重试')
          .fontSize(16)
          .fontColor('#E53935')
          .margin({ top: 40 })
      } else {
        Scroll() {
          Text(this.token)
            .fontSize(14)
            .fontColor('#333333')
            .wordBreak(WordBreak.BREAK_ALL)
            .padding(16)
            .width('85%')
            .borderRadius(8)
            .backgroundColor('#FFFFFF')
            .shadow({ radius: 4, color: '#00000010', offsetX: 0, offsetY: 2 })
        }
        .width('85%')
        .height('50%')
      }

      if (this.copyResult) {
        Text(this.copyResult)
          .fontSize(14)
          .fontColor('#4CAF50')
          .margin({ top: 16 })
      }

      Row({ space: 16 }) {
        Button('返回')
          .fontSize(16)
          .fontColor('#007DFF')
          .backgroundColor('#E3F2FD')
          .borderRadius(20)
          .padding({ left: 24, right: 24, top: 10, bottom: 10 })
          .onClick(() => {
            this.getUIContext().getRouter().back()
          })

        if (!this.hasError) {
          Button('复制Token')
            .fontSize(16)
            .fontColor('#FFFFFF')
            .backgroundColor('#007DFF')
            .borderRadius(20)
            .padding({ left: 24, right: 24, top: 10, bottom: 10 })
            .onClick(() => {
              this.copyToken()
            })
        }
      }
      .margin({ top: 40 })
    }
    .width('100%')
    .height('100%')
    .justifyContent(FlexAlign.Start)
    .alignItems(HorizontalAlign.Center)
    .backgroundColor('#F5F5F5')
  }
}
