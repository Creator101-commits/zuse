// A subagent's live transcript in the right panel: the same doc watch the chat
// uses, scoped to the chip's doc id on the root chat's device (desktop's
// `watch_subagent_doc`). Read-only; a nested chip opens in the same tab.
import { rpc } from "@/core/connection";
import { M } from "@/core/methods";
import type { SessionMessageEntry, TranscriptUpdate } from "@/core/protocol";
import type { Subscription } from "@/core/rpc";
import { state } from "@/core/store";
import { applyTranscriptFrame, TranscriptDesync } from "@/core/transcript";
import { VirtualList } from "@/features/projects/Palette";
import { openSubagent } from "@/features/workspace/store";
import { Spinner } from "@/ui/primitives";
import { createEffect, createSignal, createStore, Show, type StoreSetter } from "solid-js";
import { Entry, type EntryContext } from "./Entry";
import "./transcript.css";

/** Re-engage the bottom pin within this distance of the end (desktop STICK_THRESHOLD_PX). */
const STICK_PX = 70;

export function SubagentTranscript(props: { chatId: string; ref: string; title: string }) {
  const [data, setData] = createStore({
    messages: [] as SessionMessageEntry[],
    ready: false,
    error: null as string | null,
  });
  const setMessages: StoreSetter<SessionMessageEntry[]> = update => {
    setData(draft => { update(draft.messages); });
  };
  const [attempt, setAttempt] = createSignal(0);
  const [growth, setGrowth] = createSignal(0);

  // Pinned to the latest entry while it streams; the user's own scroll up unpins.
  let listEl: HTMLDivElement | undefined;
  const [pinned, setPinned] = createSignal(true);
  const stick = () => {
    if (pinned() && listEl) listEl.scrollTop = listEl.scrollHeight;
  };
  const onScroll = () => {
    if (!listEl) return;
    setPinned(listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight <= STICK_PX);
  };
  createEffect(() => growth(), () => queueMicrotask(stick));

  // The watch keys on the subagent doc; the root chat's device routes it, like
  // `chatRequest` but for a doc id that is not itself a chat. The compute phase
  // owns every reactive read so a reconnect or a retry resubscribes.
  createEffect(
    () =>
      [
        props.chatId,
        props.ref,
        state.chats[props.chatId]?.deviceId ?? null,
        attempt(),
        state.epoch,
        state.connection.kind === "connected",
      ] as const,
    ([, ref, device, , , live]) => {
      if (!live) {
        setData(draft => { Object.assign(draft, { ready: false, error: "Device unreachable. Reconnect to load this transcript." }); });
        return;
      }
      let ready = false;
      let retry: ReturnType<typeof setTimeout> | undefined;
      let sub: Subscription | undefined;
      const params = { chatId: ref, targetDeviceId: device };
      const restart = (message: string) => {
        sub?.cancel();
        setData(draft => { Object.assign(draft, { ready: false, error: message }); });
        retry = setTimeout(open, 600);
      };
      const open = () => {
        ready = false;
        sub = rpc.subscribe<TranscriptUpdate>(
          M.WATCH_DOC_MESSAGES,
          params,
          (update) => {
            const reset = "reset" in update;
            if (!ready && !reset) return restart("Refreshing transcript…");
            try {
              applyTranscriptFrame(setMessages, data.messages, update);
              setData(draft => { Object.assign(draft, { ready: true, error: null }); });
              ready = true;
              setGrowth(g => g + 1);
            } catch (error) {
              // Desyncs are caught before any write. Anything else may have thrown
              // mid-write, so drop the rows: the reset then remounts them.
              if (!(error instanceof TranscriptDesync)) {
                console.error("subagent transcript frame failed", error);
                setData(draft => { draft.messages = []; });
              }
              restart(error instanceof TranscriptDesync ? "Refreshing transcript…" : "Could not read this transcript. Retrying…");
            }
          },
          (error) => {
            if (error && rpc.connected) restart("Refreshing transcript…");
          },
        );
      };
      open();
      return () => {
        clearTimeout(retry);
        sub?.cancel();
      };
    },
  );

  const retry = () => {
    setData(draft => { draft.error = null; });
    setAttempt(t => t + 1);
  };

  const rootChat = () => state.chats[props.chatId];
  const ctxFor = (message: SessionMessageEntry): EntryContext => ({
    // The subagent ref, not a chat id: fork and visual lookups stay hidden.
    chatId: props.ref,
    device: () => rootChat()?.deviceId ?? null,
    harness: () => rootChat()?.config?.harness,
    last: () => data.messages[data.messages.length - 1]?.id === message.id,
    openSubagent: (ref, title) => openSubagent({ chatId: props.chatId, ref, title }),
  });

  return (
    <div class="t-root">
      <Show when={data.error}>
        {(message) => (
          <div class="t-sub-notice" role="alert">
            <span class="truncate">{message()}</span>
            <button type="button" onClick={retry}>Retry</button>
          </div>
        )}
      </Show>
      <Show
        when={data.messages.length > 0}
        fallback={
          <Show when={!data.error}>
            <Show when={data.ready} fallback={<div class="ws-empty"><Spinner label="Loading" /></div>}>
              <div class="ws-empty">No activity yet</div>
            </Show>
          </Show>
        }
      >
        <VirtualList
          id="subagent-transcript"
          class="pj-transcript"
          role="log"
          label={`Subagent transcript: ${props.title}`}
          pad={16}
          items={data.messages}
          estimate={(m) => (m.role === "user" ? 60 : 160)}
          itemKey={(m) => m.id}
          scrollEl={(el) => {
            listEl = el;
            queueMicrotask(stick);
          }}
          onScroll={onScroll}
        >
          {(message) => (
            <div class="t-row">
              <div class="t-column">
                <Entry entry={message} ctx={ctxFor(message)} />
              </div>
            </div>
          )}
        </VirtualList>
      </Show>
    </div>
  );
}
