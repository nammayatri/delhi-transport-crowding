# Demo video

The video (about 3 minutes, 1440×900) is attached to the repository's GitHub release. It is a captioned walkthrough recorded from the running app (crowding service with the live feed, plus the patched OpenTripPlanner):
1. Sangam Vihar → Nehru Place, weekday 08:30. Every option shows travel time and how full the bus usually is. The fastest bus is usually crowded; leaving at 08:42 gives seats the whole way.
2. The brief's example, Anand Vihar ISBT → Connaught Place at 08:30. With "Get there fastest" the planner recommends a crowded bus arriving 09:18; sliding to "Avoid crowds" switches it to a ride with seats arriving 09:23.
3. The network map from 05:00 to 22:00: the morning and evening peaks, the standing/crowded filter at 09:00, the 3–5 M ridership range, and the live buses from the GTFS Realtime feed, each coloured by how full it usually is right now.
4. The operator view: routes needing more buses, overloaded stretches, and a route heatmap for the morning and evening peaks.

Re-record (the crowding service must be running on :8090, with OpenTripPlanner on :8080 if `OTP_ENABLED=True`):
```
npm install && npx playwright install chromium
npm run record            # writes out/crowding-demo.mp4; DEMO_URL=... to point elsewhere
```
