/**
 * libppsdk.so — P2P SDK Rust NAPI 类型声明
 */

// ── 常量 ─────────────────────────────────────────────────────────────

export const TYPE_HEARTBEAT: number
export const TYPE_DATA: number

// ── 接口类型 ─────────────────────────────────────────────────────────

export interface CandidateInfo {
  candidateLines: string[]
  localAddresses: string[]
  stunExternalIp: string
  stunExternalPort: string
  turnRelayIp: string
  turnRelayPort: string
}

export interface IdsResponse {
  code?: number
  message?: string
  error?: string
  data?: IdsRecord[]
}

export interface IdsRecord {
  appId: string
  userId: string
  type: string
  odid: string
  token: string
}

export interface ParsedFrame {
  type: number
  payload: ArrayBuffer
}

// ── 外部接口 ─────────────────────────────────────────────────────────

export function init(configJson: string): number
export function registerIds(appId: string, userId: string, odid: string, pushToken: string): IdsResponse
export function queryIds(appId: string, userId: string): IdsResponse
export function connect(peerId: string, odid: string, isDevice?: boolean, heartbeatInterval?: number): number
export function onStateChange(cb: (state: string) => void): void
export function send(data: string | ArrayBuffer): number
export function onDataReceived(cb: (data: ArrayBuffer) => void): void
export function close(): number

// ── 内部接口 ─────────────────────────────────────────────────────────

export function generateToken(): string
export function gatherCandidates(p2pToken: string): CandidateInfo
export function iceSdpNegotiate(peerId: string, odid: string, isDevice?: boolean): number
export function encodeDataFrame(text: string): ArrayBuffer
export function encodeHeartbeatReply(): ArrayBuffer
export function parseFrame(data: ArrayBuffer): ParsedFrame
export function isStunMessage(data: ArrayBuffer): boolean
export function onLog(cb: (msg: string) => void): void
export function onConnectorStateChange(cb: (connected: boolean) => void): void

// ── Connector 信令（开发中）──────────────────────────────────────────

export function connectConnector(url: string, identifier: string, authToken: string): number
export function disconnectConnector(): number
export function isConnectorRegistered(): number
export function initiateIce(targetId: string): number
export function stopIce(): number
