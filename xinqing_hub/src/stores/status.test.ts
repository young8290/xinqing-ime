import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { StatusSnapshot } from '@/api'

const snap = (over: Partial<StatusSnapshot> = {}): StatusSnapshot => ({
  state: 'fluent',
  weather: 'sunny',
  prob: null,
  offline: true,
  paused: false,
  connected: false,
  baseline_progress: 0,
  ...over,
})

type Listener = (e: { payload: StatusSnapshot }) => void
const mocks = vi.hoisted(() => ({
  listener: null as ((e: { payload: unknown }) => void) | null,
  getStatus: vi.fn(),
  unlisten: vi.fn(),
}))

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: { getStatus: mocks.getStatus },
  events: {
    statusChanged: {
      listen: async (cb: Listener) => {
        mocks.listener = cb as (e: { payload: unknown }) => void
        return mocks.unlisten
      },
    },
  },
}))

const { useStatusStore } = await import('./status')

describe('status store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    mocks.listener = null
    mocks.getStatus.mockReset()
    mocks.unlisten.mockReset()
  })

  it('打开时取快照，之后跟随事件', async () => {
    mocks.getStatus.mockResolvedValue({ status: 'ok', data: snap() })
    const s = useStatusStore()
    await s.init()
    expect(s.snapshot?.weather).toBe('sunny')
    mocks.listener!({ payload: snap({ state: 'low', weather: 'rain' }) })
    expect(s.snapshot?.weather).toBe('rain')
  })

  it('取快照期间收到的事件比快照新，不被快照覆盖', async () => {
    mocks.getStatus.mockImplementation(async () => {
      mocks.listener!({ payload: snap({ state: 'tired', weather: 'night' }) })
      return { status: 'ok', data: snap() }
    })
    const s = useStatusStore()
    await s.init()
    expect(s.snapshot?.weather).toBe('night')
  })

  it('重复 init 只订阅一次；dispose 退订', async () => {
    mocks.getStatus.mockResolvedValue({ status: 'ok', data: snap() })
    const s = useStatusStore()
    await s.init()
    await s.init()
    expect(mocks.getStatus).toHaveBeenCalledTimes(1)
    s.dispose()
    expect(mocks.unlisten).toHaveBeenCalledTimes(1)
  })

  it('命令失败时抛出带文案键的错误', async () => {
    mocks.getStatus.mockResolvedValue({
      status: 'error',
      error: { code: 'store', message_key: 'error.generic' },
    })
    await expect(useStatusStore().init()).rejects.toMatchObject({ message_key: 'error.generic' })
  })
})
