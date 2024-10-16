TODO
====

Features
--------

* move Xe/i915 freqs reading from sysfs files to PMU
* show overall device engines load/usage
* show device power usage
* option to select which device stat graph to plot
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
