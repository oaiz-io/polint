# Walker-only observed tune edges

| package | cause | kept: edges nothing else finds |
| --- | --- | --- |
| async | this_member | 4 |
| axios | this_member | 3 |
| commander | local_member | 6 |
| commander | parameter_call | 7 |
| commander | this_member | 85 |

| package | cause | stable site | observed target |
| --- | --- | --- | --- |
| async | this_member | `internal/DoublyLinkedList.js@1460..1493` | `internal/DoublyLinkedList.js:30 insertAfter` |
| async | this_member | `internal/DoublyLinkedList.js@1572..1598` | `internal/DoublyLinkedList.js:16 removeLink` |
| async | this_member | `internal/Heap.js@1310..1343` | `internal/Heap.js:23 percUp` |
| async | this_member | `internal/Heap.js@1551..1567` | `internal/Heap.js:35 percDown` |
| axios | this_member | `lib/core/AxiosHeaders.js@5079..5096` | `lib/core/AxiosHeaders.js:216 set` |
| axios | this_member | `lib/core/AxiosHeaders.js@9440..9453` | `lib/core/AxiosHeaders.js:392 toJSON` |
| axios | this_member | `lib/core/AxiosHeaders.js@9521..9534` | `lib/core/AxiosHeaders.js:392 toJSON` |
| commander | this_member | `lib/command.js@10731..10757` | `lib/command.js:644 _registerCommand` |
| commander | this_member | `lib/command.js@11920..11958` | `lib/command.js:316 createArgument` |
| commander | this_member | `lib/command.js@12117..12143` | `lib/command.js:375 addArgument` |
| commander | this_member | `lib/command.js@14577..14605` | `lib/command.js:192 createCommand` |
| commander | local_member | `lib/command.js@14611..14640` | `lib/command.js:2559 helpOption` |
| commander | local_member | `lib/command.js@14660..14691` | `lib/command.js:359 arguments` |
| commander | local_member | `lib/command.js@14718..14758` | `lib/command.js:2218 description` |
| commander | this_member | `lib/command.js@16066..16091` | `lib/command.js:1649 _findCommand` |
| commander | this_member | `lib/command.js@16182..16220` | `lib/command.js:409 helpCommand` |
| commander | this_member | `lib/command.js@17899..17962` | `lib/command.js:512 _exitCallback` |
| commander | this_member | `lib/command.js@20206..20236` | `lib/command.js:1664 _findOption` |
| commander | this_member | `lib/command.js@20263..20292` | `lib/command.js:1664 _findOption` |
| commander | this_member | `lib/command.js@20664..20693` | `lib/command.js:2392 _initOptionGroup` |
| commander | this_member | `lib/command.js@21419..21450` | `lib/command.js:2401 _initCommandGroup` |
| commander | this_member | `lib/command.js@21632..21660` | `lib/command.js:619 _registerOption` |
| commander | this_member | `lib/command.js@21965..21999` | `lib/command.js:1664 _findOption` |
| commander | this_member | `lib/command.js@22011..22162` | `lib/command.js:953 setOptionValueWithSource` |
| commander | this_member | `lib/command.js@22230..22297` | `lib/command.js:953 setOptionValueWithSource` |
| commander | this_member | `lib/command.js@24340..24377` | `lib/command.js:585 createOption` |
| commander | this_member | `lib/command.js@24809..24831` | `lib/command.js:670 addOption` |
| commander | this_member | `lib/command.js@25949..26011` | `lib/command.js:741 _optionEx` |
| commander | this_member | `lib/command.js@35617..35640` | `lib/command.js:1127 _prepareForParse` |
| commander | this_member | `lib/command.js@35663..35704` | `lib/command.js:1001 _prepareUserArgs` |
| commander | this_member | `lib/command.js@35710..35742` | `lib/command.js:1550 _parseCommand` |
| commander | this_member | `lib/command.js@37208..37235` | `lib/command.js:1141 saveStateBeforeParse` |
| commander | this_member | `lib/command.js@37256..37286` | `lib/command.js:1158 restoreStateBeforeParse` |
| commander | this_member | `lib/command.js@45068..45098` | `lib/command.js:1649 _findCommand` |
| commander | this_member | `lib/command.js@45226..45325` | `lib/command.js:1531 _chainOrCallSubCommandHook` |
| commander | this_member | `lib/command.js@45346..45589` | `lib/command.js:1481 _chainOrCall` |
| commander | this_member | `lib/command.js@47447..47477` | `lib/command.js:1403 _checkNumberOfArguments` |
| commander | parameter_call | `lib/command.js@48852..48856` | `lib/command.js:1363 <anonymous>` |
| commander | parameter_call | `lib/command.js@48852..48856` | `lib/command.js:1603 <anonymous>` |
| commander | parameter_call | `lib/command.js@48852..48856` | `lib/command.js:1607 <anonymous>` |
| commander | this_member | `lib/command.js@49089..49119` | `lib/command.js:121 _getCommandAndAncestors` |
| commander | this_member | `lib/command.js@50336..50362` | `lib/command.js:1748 parseOptions` |
| commander | this_member | `lib/command.js@50368..50391` | `lib/command.js:1966 _parseOptionsEnv` |
| commander | this_member | `lib/command.js@50454..50481` | `lib/command.js:1996 _parseOptionsImplied` |
| commander | this_member | `lib/command.js@50625..50655` | `lib/command.js:1649 _findCommand` |
| commander | this_member | `lib/command.js@50672..50737` | `lib/command.js:1352 _dispatchSubcommand` |
| commander | this_member | `lib/command.js@50760..50782` | `lib/command.js:463 _getHelpCommand` |
| commander | this_member | `lib/command.js@50808..50830` | `lib/command.js:463 _getHelpCommand` |
| commander | this_member | `lib/command.js@51422..51448` | `lib/command.js:2621 help` |
| commander | this_member | `lib/command.js@51461..51504` | `lib/command.js:2686 _outputHelpIfRequested` |
| commander | this_member | `lib/command.js@51510..51549` | `lib/command.js:1675 _checkForMissingMandatoryOptions` |
| commander | this_member | `lib/command.js@51555..51589` | `lib/command.js:1723 _checkForConflictingOptions` |
| commander | this_member | `lib/command.js@51870..51881` | `lib/command.js:2331 name` |
| commander | this_member | `lib/command.js@51954..51978` | `lib/command.js:1428 _processArguments` |
| commander | this_member | `lib/command.js@52026..52075` | `lib/command.js:1499 _chainOrCallHooks` |
| commander | this_member | `lib/command.js@52098..52192` | `lib/command.js:1481 _chainOrCall` |
| commander | this_member | `lib/command.js@52242..52363` | `lib/command.js:1481 _chainOrCall` |
| commander | this_member | `lib/command.js@52394..52444` | `lib/command.js:1499 _chainOrCallHooks` |
| commander | this_member | `lib/command.js@52706..52728` | `lib/command.js:1649 _findCommand` |
| commander | this_member | `lib/command.js@53061..53082` | `lib/command.js:2162 unknownCommand` |
| commander | this_member | `lib/command.js@54303..54333` | `lib/command.js:121 _getCommandAndAncestors` |
| commander | this_member | `lib/command.js@55776..55806` | `lib/command.js:121 _getCommandAndAncestors` |
| commander | this_member | `lib/command.js@62145..62175` | `lib/command.js:121 _getCommandAndAncestors` |
| commander | this_member | `lib/command.js@63170..63205` | `lib/command.js:533 _exit` |
| commander | this_member | `lib/command.js@6421..6445` | `lib/command.js:192 createCommand` |
| commander | local_member | `lib/command.js@6837..6856` | `lib/command.js:359 arguments` |
| commander | this_member | `lib/command.js@6862..6888` | `lib/command.js:644 _registerCommand` |
| commander | this_member | `lib/command.js@68692..68748` | `lib/command.js:1940 error` |
| commander | local_member | `lib/command.js@6917..6948` | `lib/command.js:99 copyInheritedSettings` |
| commander | this_member | `lib/command.js@69532..69549` | `lib/command.js:203 createHelp` |
| commander | this_member | `lib/command.js@69924..69981` | `lib/command.js:1940 error` |
| commander | this_member | `lib/command.js@70666..70703` | `lib/command.js:585 createOption` |
| commander | this_member | `lib/command.js@70770..70805` | `lib/command.js:619 _registerOption` |
| commander | this_member | `lib/command.js@7602..7622` | `lib/command.js:215 configureHelp` |
| commander | this_member | `lib/command.js@77186..77203` | `lib/command.js:203 createHelp` |
| commander | this_member | `lib/command.js@77225..77263` | `lib/command.js:2475 _getOutputContext` |
| commander | this_member | `lib/command.js@79102..79140` | `lib/command.js:2475 _getOutputContext` |
| commander | this_member | `lib/command.js@79310..79340` | `lib/command.js:121 _getCommandAndAncestors` |
| commander | this_member | `lib/command.js@79502..79554` | `lib/command.js:2450 helpInformation` |
| commander | local_member | `lib/command.js@79856..79892` | `lib/command.js:2490 write` |
| commander | this_member | `lib/command.js@79950..79971` | `lib/command.js:2592 _getHelpOption` |
| commander | this_member | `lib/command.js@80045..80075` | `lib/command.js:121 _getCommandAndAncestors` |
| commander | this_member | `lib/command.js@81107..81209` | `lib/command.js:585 createOption` |
| commander | this_member | `lib/command.js@81626..81663` | `lib/command.js:2559 helpOption` |
| commander | this_member | `lib/command.js@82346..82377` | `lib/command.js:2505 outputHelp` |
| commander | this_member | `lib/command.js@82766..82820` | `lib/command.js:533 _exit` |
| commander | this_member | `lib/command.js@84449..84470` | `lib/command.js:2592 _getHelpOption` |
| commander | this_member | `lib/command.js@84583..84600` | `lib/command.js:2505 outputHelp` |
| commander | this_member | `lib/command.js@84689..84745` | `lib/command.js:533 _exit` |
| commander | parameter_call | `lib/help.js@11728..11742` | `lib/help.js:484 <anonymous>` |
| commander | parameter_call | `lib/help.js@11728..11742` | `lib/help.js:514 <anonymous>` |
| commander | parameter_call | `lib/help.js@11919..11933` | `lib/help.js:484 <anonymous>` |
| commander | parameter_call | `lib/help.js@11919..11933` | `lib/help.js:514 <anonymous>` |
| commander | this_member | `lib/help.js@13254..13309` | `lib/help.js:403 formatItemList` |
| commander | this_member | `lib/help.js@13359..13491` | `lib/help.js:417 groupItems` |
| commander | this_member | `lib/help.js@14238..14302` | `lib/help.js:403 formatItemList` |
| commander | this_member | `lib/help.js@14362..14486` | `lib/help.js:417 groupItems` |
| commander | this_member | `lib/help.js@15986..16016` | `lib/help.js:575 styleDescriptionText` |
| commander | this_member | `lib/help.js@16065..16095` | `lib/help.js:575 styleDescriptionText` |
| commander | this_member | `lib/help.js@16148..16178` | `lib/help.js:575 styleDescriptionText` |
| commander | this_member | `lib/help.js@16229..16259` | `lib/help.js:575 styleDescriptionText` |
| commander | this_member | `lib/help.js@16351..16376` | `lib/help.js:597 styleOptionText` |
| commander | this_member | `lib/help.js@16915..16942` | `lib/help.js:600 styleArgumentText` |
| commander | this_member | `lib/option.js@5709..5720` | `lib/option.js:203 name` |
| commander | this_member | `lib/option.js@5770..5781` | `lib/option.js:203 name` |
