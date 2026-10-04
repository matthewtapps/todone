import { useState } from "react"
import { PlusIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import type { Task } from "@/lib/types"

export function AddTask({
  tasks,
  onAdd,
  onFetchAll,
}: {
  tasks: Task[]
  onAdd: (id: string) => void
  onFetchAll: () => void
}) {
  const [open, setOpen] = useState(false)
  const projects = [...new Set(tasks.map((t) => t.projectName))].sort()

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" aria-expanded={open}>
          <PlusIcon data-icon="inline-start" />
          Add Task
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-80 p-0" align="end">
        <Command>
          <CommandInput placeholder="Search tasks or projects…" />
          <CommandList>
            <CommandEmpty>No hidden tasks match.</CommandEmpty>
            {projects.map((p) => (
              <CommandGroup key={p} heading={p}>
                {tasks
                  .filter((t) => t.projectName === p)
                  .map((t) => (
                    <CommandItem
                      key={t.id}
                      value={`${t.name} ${p} ${t.id}`}
                      onSelect={() => {
                        onAdd(t.id)
                        setOpen(false)
                      }}
                    >
                      {t.name}
                    </CommandItem>
                  ))}
              </CommandGroup>
            ))}
            <CommandGroup heading="More">
              <CommandItem
                value="fetch all projects"
                onSelect={() => {
                  setOpen(false)
                  onFetchAll()
                }}
              >
                Fetch Tasks From All Projects…
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
