import { useId, useState } from "react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"
import { finishLogin, postSession, startLogin } from "@/lib/api"
import { load, save } from "@/lib/storage"
import type { Session, Settings, Task } from "@/lib/types"

type Open = { open: boolean; onOpenChange: (open: boolean) => void }

type ConnectProps = {
  onOpenChange: (open: boolean) => void
  onConnected: (s: Session) => void
}

// Dialog content unmounts on close, so form state in these inner components resets on each open.
export function ConnectDialog({ open, ...props }: Open & ConnectProps) {
  return (
    <Dialog open={open} onOpenChange={props.onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <ConnectForm {...props} />
      </DialogContent>
    </Dialog>
  )
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

function ConnectForm({ onOpenChange, onConnected }: ConnectProps) {
  const [shortcode, setShortcode] = useState(() =>
    load("timesheet:shortcode", "")
  )
  const [waiting, setWaiting] = useState(false)
  const [redirect, setRedirect] = useState("")
  const [curl, setCurl] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState<"start" | "finish" | "curl" | null>(null)

  const attempt = async (kind: typeof busy, fn: () => Promise<void>) => {
    setBusy(kind)
    setError("")
    try {
      await fn()
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(null)
    }
  }

  // The tab opens before the request so the browser doesn't treat it as a blocked popup.
  const start = () =>
    attempt("start", async () => {
      const tab = window.open("about:blank", "_blank")
      try {
        const code = shortcode.trim()
        const { url } = await startLogin(code)
        save("timesheet:shortcode", code)
        if (tab) tab.location.href = url
        else window.open(url, "_blank")
        setWaiting(true)
      } catch (e) {
        tab?.close()
        throw e
      }
    })

  const finish = (e: React.FormEvent) => {
    e.preventDefault()
    attempt("finish", async () => {
      try {
        onConnected(await finishLogin(redirect.trim()))
      } catch (err) {
        setWaiting(false)
        throw err
      }
    })
  }

  const pasteCurl = (e: React.FormEvent) => {
    e.preventDefault()
    attempt("curl", async () => onConnected(await postSession(curl.trim())))
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>Connect Xero</DialogTitle>
        <DialogDescription>
          Sign in to Xero in a new tab. Each sign-in lasts about 12 minutes; if
          you're already signed in to Xero, it skips the login form.
        </DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="shortcode">Organisation Code</FieldLabel>
          <div className="flex gap-2">
            <Input
              id="shortcode"
              value={shortcode}
              onChange={(e) => setShortcode(e.target.value)}
              placeholder="!AbC12…"
              autoComplete="off"
              spellCheck={false}
              translate="no"
              className="font-mono"
            />
            <Button type="button" onClick={start} disabled={busy === "start"}>
              {busy === "start" && <Spinner data-icon="inline-start" />}
              Open Xero Sign-In
            </Button>
          </div>
          <FieldDescription>
            The part after <code translate="no">/app/</code> in your Xero
            address, starting with !
          </FieldDescription>
        </Field>
        {waiting && (
          <form onSubmit={finish} className="contents">
            <Field>
              <FieldLabel htmlFor="redirect">
                Address Xero Sent You To
              </FieldLabel>
              <div className="flex gap-2">
                <Input
                  id="redirect"
                  autoFocus
                  value={redirect}
                  onChange={(e) => setRedirect(e.target.value)}
                  placeholder="https://go.xero.com/oidc/silent.html?code=…"
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono text-xs"
                />
                <Button
                  type="submit"
                  variant="outline"
                  disabled={busy === "finish"}
                >
                  {busy === "finish" && <Spinner data-icon="inline-start" />}
                  Finish Sign-In
                </Button>
              </div>
              <FieldDescription>
                After signing in, the tab lands on a blank page. Copy that tab’s
                address here, then close the tab.
              </FieldDescription>
            </Field>
          </form>
        )}
        {error && <FieldError>{error}</FieldError>}
      </FieldGroup>
      <details className="text-sm">
        <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
          Paste a cURL instead
        </summary>
        <form onSubmit={pasteCurl} className="mt-3 flex flex-col gap-2">
          <Textarea
            aria-label="cURL command"
            value={curl}
            onChange={(e) => setCurl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) pasteCurl(e)
            }}
            rows={6}
            spellCheck={false}
            placeholder="curl 'https://go.xero.com/api/projects/…"
            className="max-h-60 font-mono text-xs"
          />
          <p className="text-xs text-muted-foreground">
            In Xero, open DevTools › Network, right-click an{" "}
            <code translate="no">/api/projects</code> request, and choose Copy ›
            Copy as cURL. This lasts about 12 minutes.
          </p>
          <Button
            type="submit"
            variant="outline"
            className="self-start"
            disabled={busy === "curl"}
          >
            {busy === "curl" && <Spinner data-icon="inline-start" />}
            Connect With cURL
          </Button>
        </form>
      </details>
      <DialogFooter>
        <Button
          type="button"
          variant="outline"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
      </DialogFooter>
    </>
  )
}

type KeywordsProps = {
  task: Task | null
  value: string
  onOpenChange: (open: boolean) => void
  onSave: (value: string) => void
}

export function KeywordsDialog(props: KeywordsProps) {
  return (
    <Dialog open={!!props.task} onOpenChange={props.onOpenChange}>
      <DialogContent>
        <KeywordsForm {...props} />
      </DialogContent>
    </Dialog>
  )
}

function KeywordsForm({ task, value, onOpenChange, onSave }: KeywordsProps) {
  const id = useId()
  const [text, setText] = useState(value)
  return (
    <form
      className="contents"
      onSubmit={(e) => {
        e.preventDefault()
        onSave(text.trim())
      }}
    >
      <DialogHeader>
        <DialogTitle>Draft Keywords for {task?.name}</DialogTitle>
        <DialogDescription>
          Drafting puts any todone bullet or meeting containing one of these
          words into {task?.name}.
        </DialogDescription>
      </DialogHeader>
      <Field>
        <FieldLabel htmlFor={id}>Keywords</FieldLabel>
        <Input
          id={id}
          autoFocus
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="wtk, warren, wtk-web…"
          autoComplete="off"
          spellCheck={false}
        />
        <FieldDescription>
          Separate with commas. Matches whole words, ignoring case.
        </FieldDescription>
      </Field>
      <DialogFooter>
        <Button
          type="button"
          variant="outline"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
        <Button type="submit">Save Keywords</Button>
      </DialogFooter>
    </form>
  )
}

type SettingsProps = {
  onOpenChange: (open: boolean) => void
  settings: Settings
  onSave: (s: Settings) => void
}

export function SettingsDialog({ open, ...props }: Open & SettingsProps) {
  return (
    <Dialog open={open} onOpenChange={props.onOpenChange}>
      <DialogContent>
        <SettingsForm {...props} />
      </DialogContent>
    </Dialog>
  )
}

function SettingsForm({ onOpenChange, settings, onSave }: SettingsProps) {
  const [hours, setHours] = useState(String(settings.hoursPerDay))
  const [ignore, setIgnore] = useState(settings.ignoreEvents)
  const [error, setError] = useState("")

  return (
    <form
      className="contents"
      onSubmit={(e) => {
        e.preventDefault()
        const h = Number(hours.trim())
        if (!(h > 0 && h <= 24))
          return setError("Enter hours between 0 and 24, like 7.6 or 8.")
        try {
          new RegExp(ignore)
        } catch {
          return setError(
            "Ignored meetings must be a valid pattern, like private|lunch."
          )
        }
        onSave({ hoursPerDay: h, ignoreEvents: ignore.trim() })
      }}
    >
      <DialogHeader>
        <DialogTitle>Settings</DialogTitle>
        <DialogDescription>
          Used for daily totals and drafting.
        </DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <Field data-invalid={error.startsWith("Enter")}>
          <FieldLabel htmlFor="hours">Hours per Day</FieldLabel>
          <Input
            id="hours"
            inputMode="decimal"
            value={hours}
            onChange={(e) => setHours(e.target.value)}
          />
        </Field>
        <Field data-invalid={error.startsWith("Ignored")}>
          <FieldLabel htmlFor="ignore">Ignored Meetings</FieldLabel>
          <Input
            id="ignore"
            value={ignore}
            spellCheck={false}
            onChange={(e) => setIgnore(e.target.value)}
            placeholder="private appointment|lunch…"
          />
          <FieldDescription>
            Meetings matching this pattern are left out of drafts.
          </FieldDescription>
        </Field>
        {error && <FieldError>{error}</FieldError>}
      </FieldGroup>
      <DialogFooter>
        <Button
          type="button"
          variant="outline"
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
        <Button type="submit">Save Settings</Button>
      </DialogFooter>
    </form>
  )
}
