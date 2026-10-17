---
title: Dashboards and chart preferences
description: Explore fixed dashboards, existing saved layouts, and account-backed chart favorites.
slug: /using-panda/dashboard-customization/
---

# Dashboards and chart preferences

R:The Drive provides dedicated dashboards for Overview, Battery, Efficiency,
Charging, and Trips. Their layout is fixed in the interface: widgets cannot be
moved, resized, added, or removed.

## Existing saved layouts

Existing personal dashboards keep their saved widgets, positions, titles,
settings, and conditional visibility. A personal dashboard with the same slug
as a system dashboard continues to take precedence for its owner.

Open **Settings > Dashboards** to browse saved dashboards. Open displays the
selected row; Export downloads its existing YAML configuration. The library
has no controls to create, edit, import, duplicate, reset, delete, restore, or
lock layouts. Old edit links and saved edit-button preferences do not enable
layout editing.

## Chart picker defaults

Chart widgets can be switched temporarily from their chart picker. To keep a
chart as the default for that widget, hover its row and click the favorite star.
The choice is stored in your account for that dashboard component and returns
after a reload or sign-in from another browser. Other chart widgets keep their
own selections.

Chart definitions have a separate editor in **Settings > Charts**. See
[Chart customization](./chart-customization.md) for personal overrides,
chart import and export, and the full-window editor. Editing a chart changes
its supported presentation and data inputs without rearranging the dashboard.

## Conditional widgets

Some widgets appear only in a particular vehicle state. The Charging dashboard,
for example, uses different widgets for plugged-in and unplugged views.
A connected vehicle in standby counts as plugged in. These views follow the
vehicle's reported state automatically; the interface has no layout scenario
editor.
