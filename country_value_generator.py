import asyncio
import random
from sqlalchemy.ext.asyncio import create_async_engine, AsyncSession
from sqlalchemy.ext.declarative import declarative_base
from sqlalchemy.future import select
from sqlalchemy.orm import sessionmaker, relationship
from sqlalchemy import Column, String, Float, ForeignKey, update, delete
from app.layers.models import Layer
from app.countries.models import Country
from app.layers.links import LayerCountryLink

DATABASE_URL = "postgresql+asyncpg://postgres:bsr7nz5unB887LnThtZ9fvhMqDFZ9@localhost:5555/postgres"

Base = declarative_base()

async def main():
    engine = create_async_engine(DATABASE_URL, echo=True)
    async_session = sessionmaker(
        bind=engine, class_=AsyncSession, expire_on_commit=False
    )

    async with async_session() as session:
        async with session.begin():
            # Delete all existing records in the LayerCountryLink table
            await session.execute(delete(LayerCountryLink))

        await session.commit()

        async with session.begin():
            countries = (await session.execute(select(Country.id))).scalars().all()
            layers = (await session.execute(select(Layer.id))).scalars().all()

            for country_id in countries:
                for layer_id in layers:
                    value = random.uniform(0, 1)  # Generate a random float between 0 and 1
                    link = LayerCountryLink(
                        country_id=country_id, layer_id=layer_id, value=value
                    )
                    session.add(link)

        await session.commit()

        # Calculate global averages and update Layer table
        async with session.begin():
            for layer_id in layers:
                result = await session.execute(
                    select(LayerCountryLink.value).filter_by(layer_id=layer_id)
                )
                values = result.scalars().all()
                global_average = sum(values) / len(values) if values else 0

                await session.execute(
                    update(Layer)
                    .where(Layer.id == layer_id)
                    .values(global_average=global_average)
                )

        await session.commit()

    await engine.dispose()

if __name__ == "__main__":
    asyncio.run(main())
