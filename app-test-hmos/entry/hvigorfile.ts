import { hapTasks } from '@ohos/hvigor-ohos-plugin'
import { HvigorPlugin, HvigorNode, HvigorTask } from '@ohos/hvigor'
import { execSync } from 'child_process'
import * as path from 'path'

function buildRustSoPlugin(): HvigorPlugin {
  return {
    pluginId: 'BuildRustSoPlugin',
    async apply(currentNode: HvigorNode): Promise<void> {
      currentNode.registerTask({
        name: 'BuildRustSo',
        run(): void {
          const scriptPath: string = path.resolve(__dirname, '..', '..', 'build-so.sh')
          console.log(`[BuildRustSo] Executing: ${scriptPath}`)
          try {
            execSync(`bash "${scriptPath}"`, { stdio: 'inherit' })
            console.log('[BuildRustSo] Rust .so build completed')
          } catch (e) {
            throw new Error(`[BuildRustSo] Failed: ${e}`)
          }
        },
        postDependencies: ['default@PreBuild'],
      } as HvigorTask)
    },
  }
}

export default {
  system: hapTasks,
  plugins: [buildRustSoPlugin()],
}
