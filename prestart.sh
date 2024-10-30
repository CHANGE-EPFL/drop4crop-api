# Run migrations
poetry run alembic upgrade head
uvicorn --host=0.0.0.0 app.main:app --workers 4
