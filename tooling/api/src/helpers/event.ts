// Copyright 2019-2021 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import { WindowLabel } from '../window'
import { invokeTauriCommand } from './tauri'
import { transformCallback } from '../tauri'
import { LiteralUnion } from 'type-fest'

export interface Event<T> {
  /** Event name */
  event: EventName
  /** The label of the window that emitted this event. */
  windowLabel: string
  /** Event identifier used to unlisten */
  id: number
  /** Event payload */
  payload: T
}

export type EventName = LiteralUnion<
  | 'tauri://update'
  | 'tauri://update-available'
  | 'tauri://update-install'
  | 'tauri://update-status'
  | 'tauri://resize'
  | 'tauri://move'
  | 'tauri://close-requested'
  | 'tauri://focus'
  | 'tauri://blur'
  | 'tauri://scale-change'
  | 'tauri://menu'
  | 'tauri://file-drop'
  | 'tauri://file-drop-hover'
  | 'tauri://file-drop-cancelled',
  string
>

export type EventCallback<T> = (event: Event<T>) => void

export type UnlistenFn = () => void

/**
 * Unregister the event listener associated with the given id.
 *
 * @ignore
 * @param eventId Event identifier
 * @returns
 */
async function _unlisten(eventId: number): Promise<void> {
  return invokeTauriCommand({
    __tauriModule: 'Event',
    message: {
      cmd: 'unlisten',
      eventId
    }
  })
}

/**
 * Emits an event to the backend.
 *
 * @param event Event name. Must include only alphanumeric characters, `-`, `/`, `:` and `_`.
 * @param [windowLabel] The label of the window to which the event is sent, if null/undefined the event will be sent to all windows
 * @param [payload] Event payload
 * @returns
 */
async function emit(
  event: string,
  windowLabel?: WindowLabel,
  payload?: unknown
): Promise<void> {
  await invokeTauriCommand({
    __tauriModule: 'Event',
    message: {
      cmd: 'emit',
      event,
      windowLabel,
      payload: typeof payload === 'string' ? payload : JSON.stringify(payload)
    }
  })
}

/**
 * Listen to an event from the backend.
 *
 * @param event Event name. Must include only alphanumeric characters, `-`, `/`, `:` and `_`.
 * @param handler Event handler callback.
 * @return A promise resolving to a function to unlisten to the event.
 */
async function listen<T>(
  event: EventName,
  windowLabel: string | null,
  handler: EventCallback<T>
): Promise<UnlistenFn> {
  return invokeTauriCommand<number>({
    __tauriModule: 'Event',
    message: {
      cmd: 'listen',
      event,
      windowLabel,
      handler: transformCallback(handler)
    }
  }).then((eventId) => {
    return async () => _unlisten(eventId)
  })
}

/**
 * Listen to an one-off event from the backend.
 *
 * @param event Event name. Must include only alphanumeric characters, `-`, `/`, `:` and `_`.
 * @param handler Event handler callback.
 * @returns A promise resolving to a function to unlisten to the event.
 */
async function once<T>(
  event: EventName,
  windowLabel: string | null,
  handler: EventCallback<T>
): Promise<UnlistenFn> {
  return listen<T>(event, windowLabel, (eventData) => {
    handler(eventData)
    _unlisten(eventData.id).catch(() => {})
  })
}

export { emit, listen, once }
