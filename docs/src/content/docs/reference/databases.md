---
title: Databases
description: Every property type of a database, its settings and its values, the conversions between types, the operators of filters, sorts, grouping, the settings of views, and the limits.
---

The rules here are the server's domain rules for databases, which check every value, filter
and view before it is kept, and work out every query. [Databases](/docs/guides/databases/)
covers how they are used.

## Property types

The `type` of a property, its settings, and its values as rows send them and take them.
Values are by property id in a row's `values`; a request may name a property by id or name.
A row without a value of a property has no entry for it, and `null` in a change clears one.

| `type` | Settings | A value | Written in an export as |
|---|---|---|---|
| `title` | | A string of up to 500 characters, without line breaks: the row's page title. Exactly one per database. | The title |
| `text` | | A string of up to 10,000 characters; line breaks and tabs are kept. | The text |
| `number` | `number_format`: `plain`, `integer`, `percent` or `currency`; `currency`, an ISO 4217 code such as `EUR`, with `currency` | Any finite number. The format is how it shows: `integer` rounds, `percent` is a hundred times the value with `%`, `0.25` as `25%`, `currency` has two decimals and the code. | `1234.5`, `1235`, `25%`, `12.50 EUR` |
| `select` | `options` | An option's id. A request may name it by name too, ignoring case. | Its name |
| `multi_select` | `options` | A list of option ids, each once, in the order given. One deleted is left out of the rows that had it. | Their names, joined by commas |
| `status` | `options`, each in a `group`: `to_do`, `in_progress` or `done` | An option's id. A new status starts with Not started, In progress and Done. | Its name |
| `date` | | `{"start": ..., "end": ...}`, or the start alone: a day, `2026-11-22`, or an RFC 3339 moment, kept in UTC. The end is optional, of the same sort, and not before the start. | `2026-11-22`, `2026-11-22 12:00 UTC`, a range with `→` |
| `person` | | A list of up to 50 member ids. Those a change adds must be members; those a row names already stay. Guests neither see, set nor clear them. | Their display names; an agent's as `Scout · via Ada`, with `(deleted)` once it is |
| `checkbox` | | `true`; unchecked is no value, and `false` clears it. | `Yes` |
| `url` | | A string of up to 2048 bytes, without spaces, starting with `http:`, `https:`, `mailto:` or `tel:`. | The URL |
| `unique_id` | `prefix`: 1 to 10 letters and digits, starting with a letter, kept in upper case | Computed: the row's number after the prefix, `"NW-42"`, or alone. At most one per database. | `NW-42` |
| `created_time` | | Computed: when the row was made, an RFC 3339 moment. | `2026-11-22 12:00 UTC` |
| `created_by` | | Computed: the id of the member who made the row. Hidden from guests. | Their display name, an agent's or integration's with whom it acts for, as for `person` |
| `last_edited_time` | | Computed: when the row's title, values, place or body last changed. | As `created_time` |
| `last_edited_by` | | Computed: who changed it last. Hidden from guests. | As `created_by` |

An option has an `id`, a `name` of 1 to 100 characters unique in its property ignoring case,
an optional `hue` of 0 to 359, grey without one, and for a status its `group`. A property has
at most 200 options. Property names are 1 to 100 characters, unique in their database
ignoring case.

## Changing a type

Changing a property's type converts each row's value, those in the trash included:

| From | To | Each value becomes |
|---|---|---|
| any | the same type | itself: only the settings change, such as a number's format or a unique id's prefix |
| a select, multi-select or status | a select, multi-select or status | the same options, by the same ids; a status takes each option's group, or `to_do` |
| any other | a select, multi-select or status | an option named after its text, made for it; a multi-select splits the text at commas |
| any | `text` | its text, as an export writes it |
| any | `number` | its text read as a number, commas and spaces between digits allowed; a checked checkbox is 1 |
| any | `checkbox` | checked for a number other than 0, or a text such as `yes`, `true`, `x` or `1`; unchecked for 0, `no`, `false` or `0` |
| any | `url`, `date` | its text read as a URL, or as a day or a moment |
| any but a person | `person` | nothing |
| any | a computed type | nothing: the new property computes its values |
| a computed type | any | nothing: computed values were never kept |

A value that becomes nothing is **lost**, and so is all but the first of several values
becoming one, as a multi-select's options becoming a select's. A change that loses values
answers `409 lossy_change`, with how many rows would lose theirs, unless it is sent with
`"confirm": true`. One that would make a row's values larger than 64 KiB answers
`409 limit_reached`. The title keeps its type, and no other property becomes a title or a
second unique id. A person's values become its people's display names, as text or as
options, which is why guests, who know no other member, do not change a person property's
type.

## Filters

A filter is a condition, or a group of filters: `{"and": [...]}` passes when all of them
pass, `{"or": [...]}` when one does. Groups nest at most three deep, with at most 50
conditions in all. A condition names a `property`, by id or name, an `operator` its type
takes, and the `value` to compare with:

```json
{"and": [
  {"property": "Status", "operator": "in_group", "value": "in_progress"},
  {"or": [
    {"property": "Owner", "operator": "contains", "value": "me"},
    {"property": "Due", "operator": "before", "value": "one_week_from_now"}
  ]}
]}
```

| Types | Operators | `value` |
|---|---|---|
| `title`, `text`, `url` | `is`, `is_not`, `contains`, `does_not_contain`, `starts_with`, `ends_with` | A string of 1 to 1000 characters, compared ignoring case |
| `number`, `unique_id` | `equals`, `not_equals`, `greater_than`, `greater_than_or_equal`, `less_than`, `less_than_or_equal` | A number; a unique id compares its row's number |
| `select` | `is`, `is_not` | An option, by id or name |
| `status` | `is`, `is_not`; `in_group` | An option; a group, `to_do`, `in_progress` or `done` |
| `multi_select` | `contains`, `does_not_contain` | An option |
| `date`, `created_time`, `last_edited_time` | `is`, `before`, `after`, `on_or_before`, `on_or_after` | A day, `2026-11-22`, or one relative to today: `today`, `tomorrow`, `yesterday`, `one_week_ago`, `one_week_from_now`, `one_month_ago`, `one_month_from_now` |
| `date`, `created_time`, `last_edited_time` | `within` | A range of days counted from today, today included: `past_week`, `past_month`, `past_year`, `next_week`, `next_month`, `next_year` |
| `person`, `created_by`, `last_edited_by` | `contains`, `does_not_contain` | A member's id, or `me`, the caller |
| `checkbox` | `is` | `true` or `false` |

Every type but the checkbox also takes `is_empty` and `is_not_empty`, without a value.

A row without a value passes the negative operators, `is_not`, `does_not_contain` and
`not_equals`, and no other; an unchecked checkbox `is` `false`. Dates compare by day: a
moment counts on the day it falls on at the caller's UTC offset, a date with an end by its
start, and a week is 7 days, a month and a year calendar ones, ending early in shorter
months. A filter only sees what the caller sees: for a guest, rows have no people, so a
condition on a person property passes as for a row without one.

## Sorts

A view, or a query, sorts by up to 10 properties, each once: `{"property": ...,
"direction": "ascending"}` or `"descending"`. Each sort orders the rows the ones before it
left equal, and the rows' manual order settles the rest.

| Types | Sorted by |
|---|---|
| `title`, `text`, `url` | The text, ignoring case |
| `number`, `unique_id` | The number |
| `select`, `status` | The option's place among the property's options |
| `multi_select` | Its options' places, the first first |
| `date`, `created_time`, `last_edited_time` | The start, a day as its midnight UTC, then the end |
| `person`, `created_by`, `last_edited_by` | The display names of its people, ignoring case |
| `checkbox` | Unchecked before checked |

Rows without a value come last, whichever the direction.

## Grouping

A view groups by a `select`, `status` or `person` property:

```json
{"property": "Status", "order": ["Done", null], "hidden": ["Not started"]}
```

A row is in the group of its value: an option, or each person it names, so a row naming two
people is in both their groups. Rows without a value are in the group `null`. The groups come
in the view's order: those `order` lists first, then the rest in their natural order, the
group of rows without a value first, then a select's or status's options in their order, or
the people by name. Every option has its group, with rows or not, and a person has one while
a row names them. `hidden` lists the groups whose rows the view leaves out. Groups are named
by an option's id or name, a person's id, or `null`; answers name them by id.

## Views

| Field | What it holds |
|---|---|
| `name` | 1 to 100 characters. |
| `layout` | `table`, `board` or `list`. A board groups: made one, or given another grouping, it names one. |
| `filter` | A filter, or `null`. |
| `sorts` | Up to 10 sorts. |
| `group` | A grouping, or `null` but on a board. |
| `properties` | Every property in the view's order: `{"property": ..., "visible": true, "width": 160}`, the width 40 to 2000 pixels, or absent until a column is resized. Properties a request leaves out follow in the schema's order, shown in a table, and elsewhere hidden but the title. |

A view keeps what it was given, checked against the schema then. When the schema changes,
what no longer fits is left out: the conditions, sorts and grouping of a deleted property, or
of one whose new type does not take their operator or value, and the groups of deleted
options. A board whose grouping property is gone shows its rows ungrouped, and takes other
changes, until it is given another.

Guests, who know no other member, read a view without the people it names: conditions that
compare with a member's id are left out, `me` kept, and so are the people a grouping by
person places or hides by hand. What the view shows them is the same, as the rows they read
name nobody. A guest who edits the database and changes a view's filter or grouping replaces
it with theirs, which names nobody either; their other changes to the view keep it as it is.

## Limits

| Limit | Value |
|---|---|
| Rows of a database, out of the trash | 10,000 |
| Properties of a database | 100 |
| Options of a property | 200 |
| Views of a database | 50 |
| Rows in one part of a query | 1 to 1000, 100 unless set |
| Conditions of a filter | 50, in groups nested 3 deep |
| Sorts of a view | 10 |
| Groups a view orders or hides | 500 |
| A text value | 10,000 characters |
| A row's values together, as JSON text | 64 KiB |
| A URL value | 2048 bytes |
| People of a person value | 50 |
