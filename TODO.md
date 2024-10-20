TODO
====

Features
--------

* [Ulisses] show overall device engines load/usage
  * move Xe/i915 freqs reading from sysfs files to PMU?
* show device power usage for igfx and dgfx (hwmon)
  * expose generic QmHwMon interface to read the data
  * integrate with app data layer and UI
* option to select which device stat graphs to plot
  * maybe add scroll view also in device area (and handle focus between areas)
* refactor UI code to support multiple screens
  * simplify and consolidate UI styles being used
  * add detailed info screen for clients
  * show graphs for mem per region and for engines

Code Quality/Structure
----------------------

* provide both lib and app crates
* improve code comments
* review
  * struct/fn naming and visibility of fields/methods
  * add more Option<> fields, store refs and not owned vals
