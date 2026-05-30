import { AbilityConstant, ConfigurationConstant, UIAbility, Want, abilityAccessCtrl, Permissions } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import { pushService, pushCommon } from '@kit.PushKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { notificationManager } from '@kit.NotificationKit';

const DOMAIN = 0x0000;

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onCreate START');
    try {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET);
    } catch (err) {
      hilog.error(DOMAIN, 'testTag', '[V2] Failed to set colorMode. Cause: %{public}s', JSON.stringify(err));
    }
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onCreate END, launchReason: %{public}d', launchParam.launchReason);

    // 请求网络相关权限（user_grant，需运行时申请）
    this.requestNetworkPermissions();

    // 请求通知权限
    this.requestNotificationPermission();

    // 检查是否由通知点击拉起
    if (want && want.parameters) {
      hilog.info(DOMAIN, 'testTag', '[V2] onCreate want params: %{public}s', JSON.stringify(want.parameters));
    }

    // 注册后台消息监听
    this.registerPushCallback();
  }

  onDestroy(): void {
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onDestroy');
  }

  private requestNetworkPermissions(): void {
    try {
      const atManager = abilityAccessCtrl.createAtManager();
      const permissions: Array<Permissions> = [
        'ohos.permission.GET_WIFI_INFO',
        'ohos.permission.GET_NETWORK_INFO',
      ];
      atManager.requestPermissionsFromUser(this.context, permissions).then((data) => {
        hilog.info(DOMAIN, 'testTag', '[V2] Network permissions: %{public}s', JSON.stringify(data));
      }).catch((err: BusinessError) => {
        hilog.error(DOMAIN, 'testTag', '[V2] Request network permissions failed: %{public}d %{public}s',
          err.code, err.message);
      });
    } catch (err) {
      const e = err as BusinessError;
      hilog.error(DOMAIN, 'testTag', '[V2] Request network permissions exception: %{public}d %{public}s',
        e.code, e.message);
    }
  }

  private requestNotificationPermission(): void {
    try {
      // 创建通知渠道
      notificationManager.addSlot(notificationManager.SlotType.OTHER_TYPES, (err: BusinessError) => {
        if (err && err.code !== 0) {
          hilog.error(DOMAIN, 'testTag', '[V2] addSlot failed: %{public}d %{public}s', err.code, err.message);
        } else {
          hilog.info(DOMAIN, 'testTag', '[V2] addSlot success');
        }
      });
      // 请求通知权限
      notificationManager.isNotificationEnabled().then((enabled: boolean) => {
        if (!enabled) {
          notificationManager.requestEnableNotification(this.context).then(() => {
            hilog.info(DOMAIN, 'testTag', '[V2] Notification permission granted');
          }).catch((err: BusinessError) => {
            hilog.error(DOMAIN, 'testTag',
              '[V2] Request notification permission failed: %{public}d %{public}s', err.code, err.message);
          });
        }
      }).catch((err: BusinessError) => {
        hilog.error(DOMAIN, 'testTag', '[V2] isNotificationEnabled failed: %{public}d %{public}s',
          err.code, err.message);
      });
    } catch (err) {
      const e = err as BusinessError;
      hilog.error(DOMAIN, 'testTag', '[V2] Request notification permission exception: %{public}d %{public}s',
        e.code, e.message);
    }
  }

  private registerPushCallback(): void {
    hilog.info(DOMAIN, 'testTag', '[V2] registerPushCallback');
    try {
      pushService.receiveMessage('BACKGROUND', this, (payload: pushCommon.PushPayload) => {
        hilog.info(DOMAIN, 'testTag', '[V2] receiveMessage, type: %{public}s, data: %{public}s',
          payload.type, payload.data);
      });
    } catch (err) {
      const e = err as BusinessError;
      hilog.error(DOMAIN, 'testTag', '[V2] registerPushCallback failed: %{public}d %{public}s', e.code, e.message);
    }
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onWindowStageCreate');

    windowStage.loadContent('pages/Index', (err) => {
      if (err.code) {
        hilog.error(DOMAIN, 'testTag', '[V2] Failed to load content. Cause: %{public}s', JSON.stringify(err));
        return;
      }
      hilog.info(DOMAIN, 'testTag', '[V2] Index page loaded successfully');
    });
  }

  onWindowStageDestroy(): void {
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onWindowStageDestroy');
  }

  onForeground(): void {
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onForeground');
  }

  onBackground(): void {
    hilog.info(DOMAIN, 'testTag', '[V2] EntryAbility onBackground');
  }
}