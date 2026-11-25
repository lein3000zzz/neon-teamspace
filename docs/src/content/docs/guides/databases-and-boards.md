---
title: Databases and boards
description: Databases in the web app, full page and inside a page; their table, board and list views; editing values in place; filters, sorts and groups; properties and their types; rows as pages; live changes; and the keyboard and touch ways to do all of it.
---

A database holds rows, and lays them out in views: a **table** of values, a **board** of cards
in columns, or a **list**. Each row is a page of its own too, with its properties at the top
and a body below, edited live like any page. This page is about the web app.
[Databases](/docs/guides/databases/) has the rules behind it and the API it calls, and [the
databases reference](/docs/reference/databases/) every property type, operator and limit.

What you see of a database is what you may do with it: someone who only views it sees its
views and values, without the ways to change them. A guest sees no people in it, as
[below](#guests). The server decides in the end; when it refuses a change, the database says
why above its views, or a property's dialog in the dialog, and shows the value as the server
keeps it:

| The notice says | When |
|---|---|
| **The database is full** | A row would be added past the 10,000 a database holds out of the trash. Moving some to the trash makes room; the trash refuses to bring a row back into a full database the same way. |
| **The change goes past a limit** | A row's values together would take more than 64 KiB, as a long text can; or the database would pass another of its [limits](/docs/reference/databases/#limits), such as 50 views. The server's own words say which. |
| **The change was not saved** | Anything else, such as a change you are no longer allowed to make once your access changed. |

**Dismiss** puts the notice away.

## Making a database

| Where | Makes |
|---|---|
| **New database** on a teamspace's page | A database at the top of that teamspace. |
| **Add a database inside**, in a page's menu in the sidebar | A database under that page, a page of its own. |
| `/database` in a page's editor | A database **inside the page**, shown where it was typed. |

A new database is Untitled, with two properties, **Name** and **Status**, and two views, a
table and a board grouped by status. It opens with the cursor in its title.

In the sidebar a database shows with a table's glyph, among the pages. Its rows do not show
there: its views are where they are. Nothing else goes under a database, so it has no way to
add a page inside it, and pages cannot be dropped onto it.

## Views

The tabs above a database are its views. Click one, or move between them with the arrow keys
once one has focus. The view in view has a menu beside its tab, for those who edit the
database:

| Item | What it does |
|---|---|
| **Rename** | Turns the tab into a field: Enter or leaving it saves, Escape keeps the old name. |
| **Duplicate** | Copies the view, with its filter, sorts, grouping and columns, right after it. |
| **Show as table**, **board** or **list** | Lays the same rows out another way. A board needs a select, status or person property to group by; the first one it finds is used. |
| **Move left**, **Move right** | Reorders the views. |
| **Delete** | Asks first. A database keeps at least one view. |

**+** after the tabs adds a view: a table, a board, or a list.

On a database's own page, the address names the view in view, so a link opens the same view.
Inside a page, the view picked is kept in the page for everyone, by those who edit the page;
those who only view it pick for themselves.

### Table

A row per line and a column per property, the title first. On a narrow screen the table
scrolls sideways and the title column stays in place.

- **A value**: click it, or reach it with Tab and press Enter, to edit it in place. Each type
  has its own editor; see [Values](#values).
- **A title**: click it to type a new one. **Open** beside it opens the row; its menu opens it
  as a full page or moves it to the trash. Both show as the pointer passes, and always on a
  touch screen.
- **A column's heading** opens its menu: **Rename…**, **Change type…**, **Sort ascending** and
  **Sort descending**, **Filter by it**, **Hide in this view**, **Insert left** and **Insert
  right** to add a property beside it, **Move left** and **Move right**, and **Delete
  property**, which asks first. The title can be neither hidden nor deleted.
- **Reorder columns** by dragging a heading onto another, or with its menu's moves.
- **Resize a column** by dragging the edge of its heading. With the keyboard, Tab to the edge,
  a slider, and press the arrow keys. The width is kept in the view once it settles.
- **+** after the last heading adds a property, shown in this view.
- **New**, under the rows, adds a row with the cursor in its title. The count beside it says
  how many rows the view shows.

A table can group its rows too, from **Group**: each group then has a heading with its count.

### Board

A column per group of the property the board groups by: each option of a status or a select,
in its order, or each person a row names, after the column of rows without a value. Columns
of options are drawn in the option's colour.

- **Move a card** by dragging it to another column, or within one, between two cards; a line
  shows where it lands, and Escape calls the drag off. Moving it to another column sets its
  value: a status's option, or, on a board of people, the person of the new column in place
  of the old one's.
- **From the keyboard or on a touch screen**, the card's menu has **Move to** each other
  column, and **Move up** and **Move down** in its own. On a phone the board scrolls sideways
  with a swipe, and cards move from their menu.
- **New**, at the foot of a column, adds a card there, its value set to the column's.
- **A column's menu** moves it left or right, an order the view keeps, or hides it. Hidden
  columns fold away at the end, with their counts, and the eye beside each shows it again.
- A card shows its title and the values the view picks in **Properties**, leaving out those
  it has none of. Its title opens the row.

A view sorted by something keeps its order: cards then move between columns, not within
one. Placing a card between two others places its row among all the rows, the order every
view without sorts shows.

### List

A line per row: its title, which opens it, and the values the view picks beside it. A list
can group its rows, as a table can.

## A view's settings

The buttons beside the tabs change the view in view, for everyone who opens it. Each opens a
panel; Escape or a click outside closes it.

| Button | What it holds |
|---|---|
| **Filter** | Which rows show. The button says how many conditions there are. |
| **Sort** | Their order. |
| **Group** | What the rows group by, a select, status or person property, and which groups show. A board always groups. |
| **Properties** | Which properties the view shows, in its order: a checkbox to show each, and arrows to move it. The pencil edits a property; **Add a property** adds one, shown here. |
| Magnifier | Finds a row by its unique id, `NW-42`, in any case, or by its number alone. |

The dot and the word beside them say whether the database's [live
changes](#live-changes) come in.

### Filters

A filter is conditions on the rows' values. **Add a filter** adds one: pick the property, the
condition its type takes, and the value. **Add a filter group** adds a group of conditions of
its own, up to three deep. The first condition of a group reads **Where**; the second picks
whether the group's conditions must all pass, **And**, or only one, **Or**.

The value fits the property:

| Property | The value |
|---|---|
| Text, title, URL | Text, compared ignoring case. |
| Number, unique id | A number. |
| Select, status, multi-select | An option. A status can also be compared by its group: **To do**, **In progress** or **Done**. |
| Date, created and edited time | A day relative to today, such as **One week ago**, or **An exact date**, picked in a date field; **is within** takes a range, such as **The past week**. |
| Person, created and edited by | **Me**, whoever looks at the view, or a member. |
| Checkbox | **Checked** or **Unchecked**. |

**is empty** and **is not empty** need no value. The filter is saved with the view once you
stop changing it for a moment, and as the panel closes; a condition still missing its value
is left out until it has one. **Clear filters** removes them all.

### Sorts

**Add a sort** sorts by a property not sorted by yet; pick its direction. The first sort
decides first, and the arrows reorder them. Each change is saved at once. Rows that sort the
same keep the order they were placed in, and rows without a value come last.

## Values

| Property | Its editor |
|---|---|
| Title, text | A field. Enter saves; in a text, Shift+Enter starts a new line. Clicking outside saves too, and Escape leaves it as it was. |
| Number | A field that takes any number, `1,250.5` too. Something else says so and is not saved. |
| Select, status | The options, found by typing; picking one closes it. |
| Multi-select | The options, each toggled on or off. |
| Date | The browser's own date field, a calendar on a phone. **End date** adds an end; **Include time** takes a time of day too. An end before the start is not saved. |
| Person | The workspace's members, found by typing, each toggled. |
| Checkbox | Checked in place. |
| URL | A field that takes `http:`, `https:`, `mailto:` and `tel:` links, with a link to open the one saved. |
| Unique id, created and edited time and by | Worked out by the server; they only show. |

In a select or a multi-select, typing a name no option has offers **Create**, which adds the
option, in the next free colour, and picks it, for those who edit the database.

Values are saved as they change. Two people changing different properties of a row at once
both keep theirs; of two changes to the same one, the last saved wins.

## Properties

**Rename…** and **Change type…** in a column's menu, the pencil in **Properties**, and a
property's name on a row's page open its settings:

| Setting | For |
|---|---|
| **Name** | Every property. Names are unique in a database, ignoring case. |
| **Type** | Every property but the title. A change that would lose values, such as text becoming numbers where some do not read as one, says how many rows would lose theirs and asks first. |
| **Number format** | A number: **Number**, **Whole number**, **Percent** or **Currency**, with its three-letter code. It changes how numbers show, not what is kept. |
| **Prefix** | A unique id: the letters before each row's number, as `NW` in `NW-42`. |
| **Date format** | A date: how its days show, **Nov 22, 2026**, **Sunday, November 22, 2026** or **2026-11-22**. This one is yours, kept in this browser; others pick their own. |
| **Options** | A select, a multi-select or a status. Each has a name, a colour from twelve hues or grey, and for a status a group. The arrows reorder them; removing one clears it from every row that had it. |

**Delete property**, in the same dialog or a column's menu, deletes the property and its value
on every row, after asking. The title stays.

## Rows

A row opens from its title in any view:

- **Beside the database**, on a wide screen: a panel over the side of the page, with the row as
  its own page shows it. **Open as full page** opens it so; **Close** or Escape goes back.
- **In place of the database**, on a narrower screen, or from a row's menu.

A row's page is a page like any other, with its icon, cover and title, and its body edited
live below. Between the title and the body are its **properties**, each editable as in the
views. A database without a unique id property shows the row's number as its **ID**. Those
who edit the database also find, on each property's name, its settings and the way to move
it up or down: that order is the one every row's page lists them in. **Add a property** adds
one to the database.

Someone a single row is shared with, without its database, sees what every row has, its id
and when it was made and last changed, then its body; not the database's properties, and
nothing on the page leads to the database.

A row goes to the trash from its menu in a view, or from its page's menu, and comes back from
the [trash](/docs/guides/editing-pages/#the-trash). It stays in its database: its page's menu
has no **Move to…**.

## Inside a page

`/database` in a page's editor, or **Database** in the menu `/` opens, makes a database inside
the page: a database of its own, kept under the page in the sidebar, and shown in the page
where it was typed. Its title is typed above its views; **Open as full page** opens it on its
own page, where everything above works too.

The block is a part of the page's document, so it moves, copies and goes away as blocks do.
Deleting the block leaves the database under the page. A block naming a database you may not
see, or one that is gone, says so.

## Live changes

Each database shows changes as they are saved: a value someone sets, a card moved, a row
added or trashed, a property or a view changed. The word beside the view's settings says how
that stands:

| State | What it means |
|---|---|
| **Connecting** | Asking the server for the database's changes. |
| **Live** | Changes others make show as they make them. |
| **Reconnecting** | The connection dropped, or the server asked for a pause. Once it is back, everything is read afresh, so nothing missed meanwhile stays missing. |
| **Disconnected** | The server ended it: your session ended, or you can no longer see the database. A notice says which, with **Reconnect**. |

Inside a page, the word is a dot, its state said to screen readers. A row's page follows its
database's changes too.

## Guests

Guests of a workspace know no other member, so a database shows them without its people: the
values of person properties, and who made and last changed each row, are left out, and a
filter compares people only with **Me**. A guest who edits the database changes everything
else as a member does, but not a person value, and not a person property's type, which would
turn its people into names. On a board grouped by person, their cards stay in their column.

## On a phone

Everything above works on a phone. Tables scroll sideways with their titles in place; boards
scroll sideways too, and cards move from their menu; every control is a full touch target.
Dates take the phone's own date picker. A row opens as a full page.
