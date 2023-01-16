# Blurber - a Discord bot to enhance Planetside 2 gameplay

This bot will play audio tracks in a voice channel based off actions you perform in game (kills, revives, stealing CTF
conduits, etc).
I made this to play with [I Think You Should Leave's Detective Crashmore](https://www.youtube.com/watch?v=zieppd4yABQ)
voicelines to add some extra amusement to my usual PS2 shenanigans, but I made it modular enough so that other
voicepacks can be added and used instead. If you want to add a voicepack, see the README in `voicepacks/`

## How to use

1. [add the Santa Claus bot to your Discord server](https://discord.com/oauth2/authorize?client_id=1055544310575149188&permissions=3147776&scope=bot%20applications.commands).
   The bot will need permissions to connect to/speak in voice channels, create application commands, and send messages.
2. connect to a voice channel that the bot can join.
3. use the /track command to begin a session. `character_name` should be the name of your Planetside character, and
   `voicepack` should be one of the voicepack options (e.g. "crashmore").

## Limitations

You can only track one character at a time per guild, since the bot can only join one VC at a time.

The bot detects in-game actions using [Daybreak's Event Streaming Service](https://census.daybreakgames.com/#what-is-websocket),
which means playing tracks is limited by what the ESS provides us 3rd party devs.

## Reporting bugs/issues

Please create a new issue in this repository describing any problems you encounter. Please provide steps to reproduce
the issue.

## Contributing

If you want to add a voicepack, see the README in `voicepacks/`

If you want to add a voiceline category, here are the general steps (you'll need to know some Rust):

1. fork this repo
2. add the empty category .txt file to all voicepacks (including `TEMPLATE`).
3. add the logic to play a random track from the category in `event.rs`, `handle_event()`.
4. if your category uses events we don't currently subscribe to, update the event names in `track.rs`,
   `character_subscription()`.
5. open a PR to the upstream repo. If the CI tests pass (WIP), I'll merge and release with your new category.
