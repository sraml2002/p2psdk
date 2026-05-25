import { resourceManager } from '@kit.LocalizationKit'
import { util } from '@kit.ArkTS'

export interface AppConfig {
  idsUrl: string
  natUrl: string
  appId: string
  userId: string
}

export function loadAppConfig(context: Context): AppConfig {
  const resMgr: resourceManager.ResourceManager = context.resourceManager
  const bytes: Uint8Array = resMgr.getRawFileContentSync('app_config.json')
  const text: string = new util.TextDecoder().decodeToString(bytes)
  const json: Record<string, Object> = JSON.parse(text) as Record<string, Object>
  return {
    idsUrl: json['idsUrl'] as string,
    natUrl: json['natUrl'] as string,
    appId: json['appId'] as string,
    userId: json['userId'] as string,
  }
}
