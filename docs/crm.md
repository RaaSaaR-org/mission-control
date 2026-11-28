# CRM: leads and events

mc can track the people you meet at trade fairs, conferences and meetups
(business cards, LinkedIn requests), move them through a pipeline, and broker
introductions to your partners and customers. Two entity kinds do this:

- **Lead** (`LEAD-NNN`, kind `leads`): a person you met. A lead is separate
  from a contact. When the relationship becomes real, `convert_lead` creates a
  contact (`CONT-NNN`) under a customer. Existing contacts are never changed.
- **Event** (`EVT-NNN`, kind `events`): where you met them. A lead points at
  its event. The event does not list its leads; you find them with a reverse
  lookup (`list_leads(event=EVT-001)`, or the event's detail page in the
  dashboard).

Both kinds work only in **standalone** repos. If `config.yml` has a `paths:`
section, list them there to turn them on (see [Configuration](#configuration)).

## Data model

### Lead: `leads/LEAD-NNN-<slug>/LEAD-NNN.md`

Each lead gets its own folder, so files about the person can sit next to it:

```
leads/LEAD-007-jane-doe/
├── LEAD-007.md     # the lead (frontmatter + body)
├── card.jpg        # optional: photo of the business card (or card.png)
└── research.md     # optional: background written by a research agent
```

mc reads only `LEAD-NNN.md`. Side files may have any name and need no
frontmatter.

```yaml
---
id: "LEAD-007"
aliases: ["LEAD-007"]
name: "Jane Doe"
slug: "jane-doe"
company: "Acme GmbH"
role: "CTO"
email: "jane@acme.example"
phone: ""
linkedin: "https://www.linkedin.com/in/janedoe"
website: "https://acme.example"
status: "new"                   # pipeline stage, see below
owner: ""
source: "event"                 # event | card | linkedin | referral | inbound | other
event: "[[EVT-001|Hannover Messe 2026]]"
introduce_to: ["[[CUST-002|Zema]]"]   # partners/customers to introduce the lead to
warmth: "hot"                   # hot | warm | cold
next_follow_up: "2026-12-01"    # YYYY-MM-DD
last_contact: "2026-11-26"      # YYYY-MM-DD, set by log_interaction
converted_to: ""                # "[[CONT-016|Jane Doe]]" once converted
tags: []
created: "2026-11-26"
updated: "2026-11-26"
---

# Jane Doe

## Summary

Who they are, what they need, why they matter to us or our partners.

## Interactions

- 2026-11-26 · call · Talked about a pilot at their plant
- 2026-11-20 · event · Met at the booth, has two UR5 cells

## Notes
```

| Field | Notes |
|---|---|
| `status` | Pipeline stage, from `statuses.lead`. The default stages, in order, are `new`, `contacted`, `qualified`, `intro-planned`, `introduced`, `converted`, `lost`. New leads start in the first configured stage. `converted` and `lost` are *closed*: closed leads never count as due for a follow-up. |
| `source` | `event`, `card`, `linkedin`, `referral`, `inbound` or `other`. It defaults to `event` when the lead is created with an event, and is empty otherwise. |
| `event` | One event, stored as `"[[EVT-NNN\|Title]]"`. The event must exist. |
| `introduce_to` | Customers (partners) to introduce the lead to, stored as `["[[CUST-NNN\|Name]]", …]`. Each must exist. |
| `warmth` | `hot`, `warm` or `cold`, or empty. |
| `next_follow_up`, `last_contact` | Strict `YYYY-MM-DD`. |
| `converted_to` | Set by `convert_lead` to `"[[CONT-NNN\|Name]]"`. Don't set it by hand. |

References are labelled wiki-links (`[[ID|Label]]`), so they show as names in
Obsidian. JSON output (MCP, REST, `mc list --json`, `data/*.json`) strips them
to bare IDs: `"event": "EVT-001"`, `"introduce_to": ["CUST-002"]`,
`"converted_to": "CONT-016"`. Like every mc entity, a lead file ends with a
`%% mc-links: … %%` footer that mirrors its frontmatter links for Obsidian's
graph view. mc maintains this footer.

**Body sections.** The lead has three sections:

- `## Summary`: who the person is.
- `## Interactions`: the log, newest first. Each line has the form
  `- YYYY-MM-DD · <type> · <note>`, where `<type>` is one of `call`, `email`,
  `meeting`, `event`, `linkedin`, `intro`, `note`.
- `## Notes`: free text.

`log_interaction` maintains the log. You can also edit it by hand, as long as
each entry stays on one line that starts with `- YYYY-MM-DD`.

### Event: `events/EVT-NNN-<slug>/EVT-NNN.md`

```yaml
---
id: "EVT-001"
aliases: ["EVT-001"]
title: "Hannover Messe 2026"
slug: "hannover-messe-2026"
status: "planned"          # planned | attended | cancelled
type: "trade-fair"         # trade-fair | conference | meetup | workshop | other (default other)
date: "2026-04-20"         # first day
end_date: "2026-04-24"     # optional, not before date
location: "Hannover"
url: "https://www.hannovermesse.de"
owner: ""
tags: []
created: "2026-11-26"
updated: "2026-11-26"
---

# Hannover Messe 2026

## Goals

## Debrief
```

The event's statuses come from `statuses.event`. Like a lead, an event has its
own folder, so you can keep files such as `booth-plan.pdf` next to it.

## Operations

Every surface calls the same shared functions in `src/commands/new.rs`
(`create_lead`, `create_event`) and `src/commands/lead.rs` (`update_lead`,
`move_lead`, `log_interaction`, `convert_lead`). Each one validates the whole
input before it writes anything, and holds the repo write lock while it reads,
changes and writes the file.

| Operation | MCP tool | REST | CLI |
|---|---|---|---|
| Create a lead | `create_lead` | `POST /v1/leads` | `mc new lead "Name" [--company --role --email --phone --linkedin --website --source --event --introduce-to --warmth --follow-up --owner --status --tags --notes]` |
| Change fields | `update_lead` | `PATCH /v1/leads/{id}` | `mc lead set <ID> [--name … --status … --tags …]` |
| Change stage | `move_lead` | `POST /v1/leads/{id}/move` | `mc lead move <ID> <stage>` |
| Log an interaction | `log_interaction` | `POST /v1/leads/{id}/interactions` | `mc lead log <ID> <type> "note" [--date] [--follow-up]` |
| Convert to a contact | `convert_lead` | `POST /v1/leads/{id}/convert` | `mc lead convert <ID> [--customer CUST-NNN \| --new-customer "Name"] [--link-existing]` |
| List leads | `list_leads` | `GET /v1/leads` | `mc list leads [--status --event --tag --owner --warmth --introduce-to --follow-up-due]` |
| Create an event | `create_event` | `POST /v1/events` | `mc new event "Title" --date YYYY-MM-DD [--end-date --location --type --url --owner --status --tags]` |
| List events | `list_entities(kind=events)` | `GET /v1/entities/events` | `mc list events [--status --tag]` |
| Read one | `get_entity`, `read_entity_file` | `GET /v1/entities/lead/{id}` | `mc show LEAD-007` |

IDs are forgiving on every surface: `lead-7`, `LEAD-0007` and (where the kind
is known) `7` all mean `LEAD-007`. Statuses and vocabularies are matched
case-insensitively, and `Intro Planned` becomes `intro-planned`.

### create_lead

- `name` is required. Everything else is optional.
- `event` and `introduce_to` must name existing entities. They are stored as
  labelled links.
- `notes` goes into the `## Notes` section.
- Returns `{id, name, path}`. `path` is the lead's folder, relative to the
  repo root.

### update_lead / move_lead

- Only the fields you pass change. An empty string clears a field, except
  `name` and `status`. A list (`introduce_to`, `tags`) replaces the whole list,
  and an empty list clears it.
- A new name also replaces the body's `# Old name` heading. The folder keeps
  its old name, and `mc validate` doesn't complain about that.
- If nothing actually changes, the file is left alone. Otherwise `updated` is
  set to today.
- When `converted` is a configured stage, it goes together with
  `converted_to`: moving a lead to `converted` without a contact is refused
  (use `convert_lead`), and a lead with `converted_to` can't leave
  `converted` (a conflict: REST `409`). The dashboard board sends a card
  dropped on the Converted lane to the lead's convert form instead.
- `last_contact` can't be set to a future date.
- Returns `{id, changed, old_status, new_status, path}`. `changed` lists the
  fields whose value actually changed.

### log_interaction(id, type, note, date?, next_follow_up?)

- Inserts `- <date> · <type> · <note>` under `## Interactions`. The newest
  entry goes first: the line is placed before the first entry dated on or
  before `date`. If the section is missing, it is created before `## Notes`.
- `date` defaults to today and can't be in the future (a future date would
  pin `last_contact`). Line breaks in `note` become spaces.
- Every type except `note` sets `last_contact` to `date`. It never moves
  backwards, so logging an older interaction leaves a newer `last_contact`
  alone.
- If the lead is still in `new`, any type except `note` moves it to
  `contacted`. This happens only when the configured stages start with `new`
  and include `contacted`.
- `next_follow_up` is updated only when you pass it. Logging an interaction
  does not clear an old follow-up.
- Returns `{id, date, type, note, line, last_contact, next_follow_up,
  old_status, new_status, path}`.

### convert_lead(id, customer?, new_customer_name?, link_existing?)

Picks the customer in this order:

1. `customer`: an existing customer ID.
2. `new_customer_name`: creates a customer with status `prospect` (if that
   status is configured) and the lead's owner. This is refused if a customer
   with that name already exists.
3. Neither: the one customer whose `name` matches the lead's `company`
   (case and spacing are ignored). This is refused if the lead has no company,
   or if no customer or several customers match.

Giving both `customer` and `new_customer_name` is an error.

The conversion then:

- creates `CONT-NNN` under the customer, with the lead's name, role, email,
  phone and tags. The contact's `## Notes` gets
  `Converted from [[LEAD-NNN|Name]] on <date>.`, plus the company, LinkedIn
  and website.
- sets the lead's `converted_to: "[[CONT-NNN|Name]]"` and `status: converted`
  (if configured), logs `- <today> · note · Converted to [[CONT-…]] at
  [[CUST-…]]`, and updates `updated`.
- refuses a lead that already has `converted_to` (a conflict: REST `409`).
- refuses (`409`) when the customer already has a contact for this person:
  the same email (case-insensitive) or, failing that, the same name. With
  `link_existing` (CLI `--link-existing`) the lead is linked to that contact
  instead (`converted_to`, status, a `Linked to …` log line); the existing
  contact is not changed.
- checks the contact and customer templates before writing anything. If a
  later step fails, it removes the contact (and a customer it created) again,
  so a retry doesn't leave a duplicate.
- returns `{id, contact, contact_path, contact_created, customer,
  customer_created, old_status, new_status, path}`.

### list_leads

All filters combine with AND:

| Filter | Matches |
|---|---|
| `status` | The pipeline stage. |
| `event` | Leads met at that event (a loose ID works). |
| `tag` | Leads with that tag. |
| `owner` | Leads with that owner. |
| `warmth` | Leads with that warmth. |
| `introduce_to` | Leads to be introduced to that customer. |
| `follow_up_due=true` | Open leads (not `converted` or `lost`) whose `next_follow_up` is today or earlier. |

Results are sorted by ID. With `follow_up_due`, they are sorted oldest
follow-up first. Each item is the lead's frontmatter with bare IDs, plus
`_kind` and `_source`.

## Validation, index, search

- `mc validate` / `validate_repo` checks lead and event folders:
  - folder names follow `PREFIX-NNN-slug`;
  - the ID file exists and has the `name` (lead) or `title` (event) field;
  - the status is configured;
  - the slug matches the folder.
- References in `event`, `introduce_to` and `converted_to` must resolve, or
  validation warns. Values of `source`, `warmth` and the event `type` outside
  their vocabularies are `field-value` warnings, and so are malformed dates.
  A `converted` status without `converted_to`, or `converted_to` on a lead in
  another stage, is a `lead-conversion` warning.
  Side files such as `research.md` are ignored.
- `mc index` / `build_index` writes `data/leads.json` and `data/events.json`,
  and adds both kinds to `data/index.json`.
- Leads and events appear in `mc status` and `get_status`, in the dashboard
  search and command palette, and in the hover previews. The sidebar shows
  them under **Network**, at `/leads` and `/events` (`g n` jumps to leads).
  Related-entity sections show the leads on a customer's page.

## Dashboard (`mc serve`)

- **`/leads`** is the pipeline board: one lane per lead status, with a count
  and a share-of-pipeline meter. Converted and lost lanes are narrow and show
  the newest eight. Cards show name, role and company, warmth, the event, a
  follow-up badge (overdue in red, today in amber) and the first partners in
  `introduce_to`. Drag a card to another lane, or use its move menu, to change
  the stage (Undo in the toast). Filters: search, event, warmth, owner, tag and
  "Follow-up due" (`?due=1`). The URL keeps them, so `/leads?event=EVT-001`
  is a shareable board for one event.
- **`/leads/list`** is the same pipeline as a table, sortable by name,
  company, stage, warmth, event, next follow-up and last contact.
- **Lead page** (`/entity/LEAD-NNN`): contact links (mail, phone, LinkedIn,
  website), a clickable stage route (with "Mark lost"), the key facts, a
  composer to log an interaction with quick follow-up dates, the interaction
  timeline, the business card when `card.*` exists, the "Introduce to" editor,
  and `research.md` (or a hint to run `/enrich-lead LEAD-NNN`). Edit (`e`),
  set follow-up and Convert (to a contact under a matched, existing or new
  customer) open in place.
- **`/leads/new`** is a phone-friendly quick capture. Only the name is
  required; "Met at" defaults to the ongoing event, else the latest one that
  started, else the next planned one. With "Add another" ticked the form
  stays open and keeps the event and warmth. "New lead" is also in the header
  of the board and in the command palette.
- **`/events`** lists upcoming and past events with a stage bar per event;
  `/events/new` creates one. An event's page shows who you met there: lead,
  conversion, intro and follow-up counts, a stage breakdown and a link to the
  board filtered to that event.
- **Overview**: a "Follow-ups" panel (overdue, today, next 7 days) and a
  "Pipeline" panel (a funnel per stage and the intros in progress).

Writes go through the dashboard's JSON API (`POST /api/leads`,
`PATCH /api/leads/{id}`, `POST /api/leads/{id}/move`, `/interactions`,
`/convert`, `POST /api/events`), which calls the same commands as the CLI and
is protected like every other dashboard write. With `--read-only` (or behind
`--base-path` without `--allow-edits`) the pages render without edit controls.

## Configuration

These are the defaults. You only need to set what differs.

```yaml
paths:
  leads: leads/        # needed when the config has a paths: section
  events: events/
id_prefixes:
  lead: LEAD
  event: EVT
statuses:
  lead: [new, contacted, qualified, intro-planned, introduced, converted, lost]
  event: [planned, attended, cancelled]
```

- In a standalone repo **without** a `paths:` section, every kind is
  available. **With** a `paths:` section, as in the HQ repo, add `leads:` and
  `events:` to it, or mc reports "leads are not enabled in this repo".
- Embedded (`.mc/`) repos have no leads or events.
- `mc init` writes `templates/lead.md` and `templates/event.md`. Repos created
  before CRM support need no migration: if the templates are missing, mc uses
  built-in copies (the constants `TEMPLATE_LEAD` and `TEMPLATE_EVENT` in
  `src/commands/init.rs`). Custom templates may reorder fields or add
  sections. mc fills in the fields it knows and keeps the rest.
- Lead conversion uses the repo's `templates/contact.md` (and
  `templates/customer.md` for a new customer), the same as `mc new contact`.
